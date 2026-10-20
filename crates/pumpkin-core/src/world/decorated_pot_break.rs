//! Decorated-pot destruction claims terrain and face/content stock before callbacks.

use std::sync::Arc;

use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::DecoratedPotLikeProperties,
    data_component_impl::CanBreakImpl,
    item_stack::ItemStack,
    tag::{self, Taggable},
    world::WorldEvent,
};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{
    VarInt,
    bedrock::client::level_event::{CLevelEvent, LevelEvent},
    java::client::play::CWorldEvent,
};
use pumpkin_util::{GameMode, math::position::BlockPos};
use pumpkin_world::world::BlockFlags;

use super::{
    World,
    loot::{LootContextParameters, collect_block_entity_components_from_entity},
    neighbor_updater::UPDATE_LIMIT,
    portal::frame_effects::{RetiredBlockEffects, RetiredBlockEntity},
};
use crate::{
    block::{blocks::decorated_pot::player_cracks_pot, entities::BlockEntity},
    entity::{
        EntityBase,
        player::Player,
        projectile::{
            arrow::ArrowEntity, egg::EggEntity, fireball::FireballEntity,
            firework_rocket::FireworkRocketEntity, trident::TridentEntity,
        },
    },
    net::ClientPlatform,
};

pub(super) struct DecoratedPotBreak<'a> {
    expected: BlockStateId,
    entity: Option<Arc<dyn BlockEntity>>,
    terrain: Option<pumpkin_world::level::SyncChunk>,
    projectile: Option<&'a dyn EntityBase>,
}

impl World {
    pub(super) fn prepare_pot_break(
        &self,
        position: &BlockPos,
        expected: BlockStateId,
        unprepared: bool,
    ) -> Option<DecoratedPotBreak<'_>> {
        (unprepared && expected.to_block() == &Block::DECORATED_POT)
            .then(|| DecoratedPotBreak::capture(self, position, expected, None))
    }

    pub(crate) fn break_decorated_pot(
        self: &Arc<Self>,
        position: &BlockPos,
        expected: BlockStateId,
        projectile: &dyn EntityBase,
    ) {
        if expected.to_block() != &Block::DECORATED_POT
            || self.get_block_state_id(position) != expected
            || !may_break(self, position, projectile)
        {
            return;
        }
        let preparation = DecoratedPotBreak::capture(self, position, expected, Some(projectile));
        if self
            .break_block_with_limit_prepared(
                position,
                None,
                BlockFlags::NOTIFY_ALL,
                UPDATE_LIMIT,
                Some(&preparation),
            )
            .is_some()
        {
            // Level.destroyBlock emits BLOCK_DESTROY only for a successful removal.
            self.emit_game_event(
                pumpkin_data::game_event::GameEvent::BlockDestroy.name(),
                position.to_centered_f64(),
            );
        }
    }

    pub(super) fn explode_decorated_pot(
        self: &Arc<Self>,
        position: &BlockPos,
        expected: BlockStateId,
        params: &LootContextParameters,
        drop_items: bool,
    ) -> Option<Vec<ItemStack>> {
        let preparation = DecoratedPotBreak::capture(self, position, expected, None);
        let retired = preparation.claim_removal(self, position, BlockStateId::AIR)?;
        if let Some(entity) = &retired.live {
            super::container_lifecycle::retire_block_entity(entity);
        }
        let drops = if drop_items {
            pot_loot(self, position, retired.live.as_deref(), params)
        } else {
            Vec::new()
        };
        finish_removal(
            self,
            position,
            expected,
            BlockStateId::AIR,
            retired,
            BlockFlags::NOTIFY_ALL,
            UPDATE_LIMIT,
        );
        Some(drops)
    }
}

impl<'a> DecoratedPotBreak<'a> {
    pub(super) fn capture(
        world: &World,
        position: &BlockPos,
        expected: BlockStateId,
        projectile: Option<&'a dyn EntityBase>,
    ) -> Self {
        Self {
            expected,
            terrain: world
                .level
                .loaded_chunks
                .get(&position.chunk_position())
                .map(|chunk| chunk.clone()),
            entity: world.get_block_entity(position),
            projectile,
        }
    }

    pub(super) fn break_admitted(
        &self,
        world: &Arc<World>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        drop_items: bool,
        flags: BlockFlags,
        update_limit: u32,
    ) -> Option<BlockStateId> {
        if self
            .projectile
            .is_some_and(|projectile| !may_break(world, position, projectile))
        {
            return None;
        }
        let tool = cause.map(|player| player.inventory().held_item());
        let replacement =
            crate::block::blocks::growing_plant::fluid_state_after_break(self.expected);
        let retired = self.claim_removal(world, position, replacement)?;
        if let Some(entity) = &retired.live {
            super::container_lifecycle::retire_block_entity(entity);
        }
        let mut properties = DecoratedPotLikeProperties::from_state_id(self.expected);
        // DecoratedPotBlock.playerWillDestroy/onProjectileHit changes the loot state.
        properties.cracked |= self.projectile.is_some()
            || tool
                .as_ref()
                .is_some_and(|tool| player_cracks_pot(tool, retired.live.as_deref()));
        let broken = properties.to_state_id(&Block::DECORATED_POT);
        if drop_items {
            let mut params = LootContextParameters {
                tool,
                block_state: Some(broken.to_state()),
                position: Some(position.to_f64()),
                killed_by_player: Some(cause.is_some()),
                ..Default::default()
            };
            if let Some(projectile) = self.projectile {
                // Block.getDrops receives the breaker as THIS_ENTITY, separately from TOOL.
                params.this_entity = Some(projectile.get_entity().entity_type);
                params.this_entity_state = super::loot::snapshot_entity(
                    projectile,
                    &mut std::collections::HashSet::new(),
                    0,
                );
            }
            for stack in pot_loot(world, position, retired.live.as_deref(), &params) {
                world.drop_stack(position, stack);
            }
            crate::block::drop_experience(
                world,
                &Block::DECORATED_POT,
                position,
                params.tool.as_ref(),
            );
        }
        finish_removal(
            world,
            position,
            broken,
            replacement,
            retired,
            flags - BlockFlags::SKIP_DROPS,
            update_limit,
        );
        send_destroy_effect(world, position, broken, cause);
        Some(broken)
    }

    fn claim_removal(
        &self,
        world: &World,
        position: &BlockPos,
        replacement: BlockStateId,
    ) -> Option<RetiredBlockEntity> {
        let _mutation = world.reserve_block_entity_mutation(*position)?;
        let mut loads = world
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = world
            .block_entities
            .get(&position.chunk_position())
            .and_then(|entities| entities.get(position).cloned());
        let same_terrain = world
            .level
            .loaded_chunks
            .get(&position.chunk_position())
            .zip(self.terrain.as_ref())
            .is_some_and(|(current, expected)| Arc::ptr_eq(current.value(), expected));
        if !same_terrain
            || !same_entity(self.entity.as_ref(), current.as_ref())
            || (current.is_none() && world.has_pending_block_entity_image(position))
        {
            // LevelChunk.getBlockEntity may lose promotion to an identical pending-NBT
            // rewrite. No live instance then is not proof that face/content stock is absent.
            return None;
        }
        // The existing publication gate ties identity, terrain CAS and saved/live retirement.
        world.write_block_state_if(position, replacement, |state| state == self.expected)?;
        Some(world.remove_loaded_block_entity_with_gate(position, &mut loads))
    }
}

fn pot_loot(
    world: &Arc<World>,
    position: &BlockPos,
    entity: Option<&dyn BlockEntity>,
    base: &LootContextParameters,
) -> Vec<ItemStack> {
    let Some(table) = world.get_loot_table("minecraft:blocks/decorated_pot") else {
        return Vec::new();
    };
    let params = LootContextParameters {
        world: Some(world.clone()),
        registry: world
            .server
            .upgrade()
            .map(|server| server.datapack_manager.clone()),
        block_entity_components: entity.map_or_else(
            Default::default,
            collect_block_entity_components_from_entity,
        ),
        ..base.clone()
    };
    crate::block::block_drops_with_context(world, position, &table, &params)
}

fn finish_removal(
    world: &Arc<World>,
    position: &BlockPos,
    broken: BlockStateId,
    replacement: BlockStateId,
    retired: RetiredBlockEntity,
    flags: BlockFlags,
    update_limit: u32,
) {
    let effects = RetiredBlockEffects::capture(world, position, broken);
    // LevelChunk.setBlockState skips removal side effects, but still retires the entity.
    if !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK) {
        retired.on_block_replaced(world, position);
    }
    // Effects use the already-retired instance and never write terrain over a callback replacement.
    world.on_block_state_set_with_block_entity(
        position,
        broken,
        replacement,
        flags,
        update_limit,
        Some(&effects),
    );
}

fn send_destroy_effect(
    world: &Arc<World>,
    position: &BlockPos,
    broken: BlockStateId,
    cause: Option<&Arc<Player>>,
) {
    let java = CWorldEvent::new(
        WorldEvent::ParticlesDestroyBlock as i32,
        *position,
        broken.as_u16().into(),
        false,
    );
    let bedrock = CLevelEvent {
        event_id: VarInt(LevelEvent::ParticlesDestroyBlock as i32),
        position: position.to_centered_f64().to_f32_lossy(),
        data: VarInt(BlockState::to_be_network_id(broken) as i32),
    };
    if let Some(player) = cause {
        if let ClientPlatform::Bedrock(client) = player.client.as_ref() {
            client.try_enqueue_client_packet(&bedrock);
        }
        world.broadcast_to_chunk_except_editioned(
            position.chunk_position(),
            &[player.get_entity().entity_uuid],
            &java,
            &bedrock,
        );
    } else {
        world.broadcast_to_chunk_editioned(position.chunk_position(), &java, &bedrock);
    }
}

fn same_entity(a: Option<&Arc<dyn BlockEntity>>, b: Option<&Arc<dyn BlockEntity>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

// Projectile.mayInteract / mayBreak / canBreakBlockInAdventureMode (26.3).
fn may_break(world: &World, position: &BlockPos, projectile: &dyn EntityBase) -> bool {
    if projectile.projectile_state().is_none()
        || !projectile
            .get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_IMPACT_PROJECTILES)
        || !world
            .level_info
            .load()
            .game_rules
            .projectiles_can_break_blocks
    {
        return false;
    }
    let owner = projectile.projectile_owner();
    if let Some(player) = owner.as_ref().and_then(|owner| owner.get_player()) {
        // ServerPlayer.mayInteract -> ServerLevel.mayInteract: no reach check for a projectile.
        if world.is_in_spawn_protection(player, position)
            || !world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(f64::from(position.0.x), f64::from(position.0.z))
        {
            return false;
        }
        if player.gamemode.load() != GameMode::Adventure {
            return true;
        }
    } else if owner.is_some() {
        if !world.level_info.load().game_rules.mob_griefing {
            return false;
        }
    } else if world.server.upgrade().is_none_or(|server| {
        server
            .defaultgamemode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .gamemode
            != GameMode::Adventure
    }) {
        return true;
    }
    projectile_can_break(projectile).is_some_and(|predicate| match &predicate.predicate {
        NbtTag::List(predicates) => predicates
            .iter()
            .any(|predicate| crate::entity::block_action::matches(predicate, world, position)),
        predicate => crate::entity::block_action::matches(predicate, world, position),
    })
}

/// Use the retained item's `CAN_BREAK`, never the owner's current hand.
/// `AbstractArrow`'s separately captured vanilla predicate is not modeled by this fork.
fn projectile_can_break(projectile: &dyn EntityBase) -> Option<CanBreakImpl> {
    let any = projectile.cast_any();
    if let Some(arrow) = any.downcast_ref::<ArrowEntity>() {
        return arrow
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_data_component::<CanBreakImpl>()
            .cloned();
    }
    if let Some(trident) = any.downcast_ref::<TridentEntity>() {
        return trident
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_data_component::<CanBreakImpl>()
            .cloned();
    }
    if let Some(egg) = any.downcast_ref::<EggEntity>() {
        return egg
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_data_component::<CanBreakImpl>()
            .cloned();
    }
    if let Some(fireball) = any.downcast_ref::<FireballEntity>() {
        return fireball
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_data_component::<CanBreakImpl>()
            .cloned();
    }
    // Impacts without a retained source item have no adventure predicate.
    any.downcast_ref::<FireworkRocketEntity>()
        .and_then(FireworkRocketEntity::block_break_predicate)
}
