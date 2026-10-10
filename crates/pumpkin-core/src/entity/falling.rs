use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::BlockState;
use pumpkin_data::BlockStateId;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_protocol::bedrock::client::CUpdateBlock;
use pumpkin_protocol::java::client::play::CBlockUpdate;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    block::{
        blocks::{anvil::AnvilBlock, dripstone::DripstoneBlock, falling::FallingBlock},
        registry::can_replace_with_other_block,
    },
    entity::{Entity, EntityBase, living::LivingEntity},
    server::Server,
    world::World,
};

pub struct FallingEntity {
    entity: Entity,
    block_state_id: AtomicCell<BlockStateId>,
    fall_distance: AtomicCell<f64>,
    hurt_entities: AtomicCell<Option<(f32, i32)>>,
    cancel_drop: AtomicBool,
}

impl FallingEntity {
    pub const fn new(entity: Entity, block_state_id: BlockStateId) -> Self {
        Self {
            entity,
            block_state_id: AtomicCell::new(block_state_id),
            fall_distance: AtomicCell::new(0.0),
            hurt_entities: AtomicCell::new(None),
            cancel_drop: AtomicBool::new(false),
        }
    }

    /// Replaces a block with a falling entity and returns the spawned entity.
    pub fn replace_spawn(
        world: &Arc<World>,
        position: BlockPos,
        block_state: BlockStateId,
    ) -> Arc<Self> {
        // FallingBlockEntity.fall leaves the source fluid and strips WATERLOGGED from the entity.
        let state = block_state.to_state();
        let source = if state.is_waterlogged() {
            Block::WATER.default_state.id
        } else {
            Block::AIR.default_state.id
        };
        let block_state = state
            .set_waterlogged(false)
            .map_or(block_state, |state| state.id);
        world.set_block_state(&position, source, BlockFlags::NOTIFY_ALL);

        let position = position.0.to_f64().add_raw(0.5, 0.0, 0.5);
        let entity = Entity::new(world.clone(), position, &EntityType::FALLING_BLOCK);
        entity
            .data
            .store(i32::from(block_state.as_u16()), Ordering::Relaxed);
        let entity = Arc::new(Self::new(entity, block_state));
        if Block::from_state_id(block_state).has_tag(&tag::Block::MINECRAFT_ANVIL) {
            AnvilBlock::falling(&entity);
        }
        world.spawn_entity_non_save(entity.clone());
        entity
    }

    /// Configures `FallingBlockEntity.setHurtsEntities` for the next landing.
    pub fn set_hurts_entities(&self, damage_per_distance: f32, damage_max: i32) {
        self.hurt_entities
            .store(Some((damage_per_distance, damage_max)));
    }

    pub(crate) fn reset_fall_distance(&self) {
        self.fall_distance.store(0.0);
    }

    // FallingBlockEntity.causeFallDamage: only live, non-creative, non-spectator living targets.
    fn cause_fall_damage(&self, distance: f64, random: &mut impl rand::Rng) {
        const ANVIL_DAMAGE_CHANCE: f32 = 0.05;
        let Some((damage_per_distance, damage_max)) = self.hurt_entities.load() else {
            return;
        };
        let distance = (distance - 1.0).ceil() as i32;
        if distance < 0 {
            return;
        }
        let state_id = self.block_state_id.load();
        let block = Block::from_state_id(state_id);
        let is_anvil = block.has_tag(&tag::Block::MINECRAFT_ANVIL);
        let damage_type = if is_anvil {
            DamageType::FALLING_ANVIL
        } else if block.has_tag(&tag::Block::MINECRAFT_SPELEOTHEMS) {
            DamageType::FALLING_STALACTITE
        } else {
            DamageType::FALLING_BLOCK
        };
        let damage =
            ((distance as f32 * damage_per_distance).floor() as i32).min(damage_max) as f32;
        let world = self.entity.world.load();
        for target in world.get_all_at_box(&self.entity.bounding_box.load()) {
            if target.is_spectator()
                || target
                    .get_player()
                    .is_some_and(crate::entity::player::Player::is_creative)
            {
                continue;
            }
            if let Some(living) = target.get_living_entity()
                && !living.dead.load(Ordering::Relaxed)
                && living.health.load() > 0.0
                && !target.get_entity().is_removed()
            {
                target.damage_with_context(
                    target.as_ref(),
                    damage,
                    damage_type,
                    None,
                    Some(self),
                    Some(self),
                );
            }
        }
        if is_anvil
            && damage > 0.0
            && random.random::<f32>() < ANVIL_DAMAGE_CHANCE + distance as f32 * ANVIL_DAMAGE_CHANCE
        {
            if let Some(state) = AnvilBlock::damage(state_id) {
                self.block_state_id.store(state);
            } else {
                self.cancel_drop.store(true, Ordering::Relaxed);
            }
        }
    }

    fn cause_fall_damage_on_landing(&self, distance: f64) {
        // Blocks.java:722/2729 hardcodes fallDistanceReduction for beds and shelf mushrooms.
        const FALL_DISTANCE_REDUCTION: f64 = 0.5;
        let world = self.entity.world.load();
        let (block, state) = world.get_block_and_state(&self.entity.get_pos_with_y_offset(0.2).0);
        // Entity.checkFallDamage / PowderSnowBlock.fallOn / SlimeBlock.fallOn.
        if block == &Block::POWDER_SNOW
            || (block == &Block::SLIME_BLOCK && self.entity.is_sneaking())
        {
            return;
        }
        let distance = if block == &Block::POINTED_DRIPSTONE {
            DripstoneBlock::stalagmite_fall_distance(state.id, distance).unwrap_or(distance)
        } else if block.has_tag(&tag::Block::MINECRAFT_BEDS) || block == &Block::SHELF_MUSHROOM {
            distance * (1.0 - FALL_DISTANCE_REDUCTION)
        } else {
            distance
        };
        // FallingBlockEntity.causeFallDamage ignores Block.fallOn's damage multiplier and source.
        self.cause_fall_damage(distance, &mut rand::rng());
    }

    // AnvilBlock.onLand/onBrokenAfterFall and SpeleothemBlock.onBrokenAfterFall.
    fn landing_event(&self, broken: bool, pos: BlockPos) {
        if self.entity.is_silent() {
            return;
        }
        let block = Block::from_state_id(self.block_state_id.load());
        let event = if block.has_tag(&tag::Block::MINECRAFT_ANVIL) {
            Some(if broken {
                WorldEvent::SoundAnvilBroken
            } else {
                WorldEvent::SoundAnvilLand
            })
        } else if broken && block == &Block::POINTED_DRIPSTONE {
            Some(WorldEvent::SoundPointedDripstoneLand)
        } else {
            None
        };
        if let Some(event) = event {
            self.entity.world.load().sync_world_event(event, pos, 0);
        }
    }

    fn land(&self) {
        let entity = &self.entity;
        entity
            .velocity
            .store(entity.velocity.load().multiply(0.7, -0.5, 0.7));
        let world = entity.world.load();
        let landing_pos = self.entity.block_pos.load();
        let (current_block, current_state) = world.get_block_and_state(&landing_pos);
        // Vanilla waits until a piston has finished moving a block through the landing cell.
        if current_block != &Block::MOVING_PISTON {
            if self.cancel_drop.load(Ordering::Relaxed) {
                entity.remove();
                self.landing_event(true, landing_pos);
                return;
            }
            let mut state_id = self.block_state_id.load();
            let block = Block::from_state_id(state_id);
            let (below_block, below_state) = world.get_block_and_state(&landing_pos.down());
            let may_replace = can_replace_with_other_block(current_block, current_state);
            let would_continue_falling = FallingBlock::can_fall_through(below_state, below_block);
            let would_survive = world.block_registry.can_place_at(
                None,
                Some(&**world),
                &**world,
                None,
                block,
                state_id.to_state(),
                &landing_pos,
                None,
                None,
            ) && !would_continue_falling;
            if may_replace && would_survive {
                // FallingBlockEntity.tick restores WATERLOGGED only in source water.
                let (fluid, fluid_state) = world.get_fluid_and_fluid_state(&landing_pos);
                if fluid.matches_type(&pumpkin_data::fluid::Fluid::WATER) && fluid_state.is_source {
                    state_id = state_id
                        .to_state()
                        .set_waterlogged(true)
                        .map_or(state_id, |state| state.id);
                }
                if block.has_tag(&tag::Block::MINECRAFT_CONCRETE_POWDERS)
                    && FallingBlock::should_solidify(&**world, &landing_pos)
                    && let Some(name) = block.name.strip_suffix("_powder")
                    && let Some(concrete) = Block::from_name(name)
                {
                    state_id = concrete.default_state.id;
                }
                // wrap FallingBlockEntity.tick's final resolved placement.
                let mut event = crate::plugin::api::events::entity::entity_change_block::EntityChangeBlockEvent::new(
                        entity.entity_id,
                        landing_pos,
                        Block::from_state_id(state_id).name.to_string(),
                    );
                if let Some(server) = world.server.upgrade() {
                    server.plugin_manager.fire_blocking(&server, &mut event);
                }
                if event.cancelled {
                    entity.remove();
                    return;
                }
                world.set_block_state(&landing_pos, state_id, BlockFlags::NOTIFY_ALL);
                // block updates to watchers before the despawn, else a invisible block gap until the tick flush.
                let placed = world.get_block_state_id(&landing_pos);
                world.send_to_tracking_players_editioned(
                    entity,
                    &CBlockUpdate::new(landing_pos, i32::from(placed.as_u16()).into()),
                    &CUpdateBlock::new(landing_pos, BlockState::to_be_network_id(placed)),
                );
                entity.remove();
                self.landing_event(false, landing_pos);
            } else {
                entity.remove();
                if world.level_info.load().game_rules.entity_drops
                    && let Some(item) = Item::from_id(block.item_id)
                {
                    self.landing_event(true, landing_pos);
                    entity.spawn_at_location(ItemStack::new(1, item));
                }
            }
        }
    }
}

impl EntityBase for FallingEntity {
    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        let entity = &self.entity;
        let old_y = entity.pos.load().y;
        let mut velocity = entity.velocity.load();
        velocity.y -= self.get_gravity();
        entity.velocity.store(velocity);
        entity.move_entity(caller, velocity);
        // Entity.checkFallDamage includes the final downward movement before the landing callback.
        let distance = if entity.touching_water.load(Ordering::Relaxed) {
            self.fall_distance.load()
        } else {
            self.fall_distance.load() + f64::from((old_y - entity.pos.load().y).max(0.0) as f32)
        };
        self.fall_distance.store(distance);
        if entity.on_ground.load(Ordering::Relaxed) {
            self.cause_fall_damage_on_landing(distance);
            self.reset_fall_distance();
        }
        entity.tick_block_collisions(caller);
        if entity.on_ground.load(Ordering::Relaxed) {
            self.land();
        }
        entity
            .velocity
            .store(entity.velocity.load().multiply(0.98, 0.98, 0.98));
    }

    fn init_data_tracker(&self) {
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::falling_block::START_POS,
            self.entity.block_pos.load(),
        );
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn can_hit(&self) -> bool {
        !self.entity.is_removed()
    }

    fn damage(&self, _caller: &dyn EntityBase, _amount: f32, _damage_type: DamageType) -> bool {
        false
    }

    fn get_gravity(&self) -> f64 {
        0.04
    }

    // TODO: Bedrock spawn metadata lacks the block variant (renders grey while falling)
    fn bedrock_y_offset(&self) -> f64 {
        0.49
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "falling_tests.rs"]
mod tests;
