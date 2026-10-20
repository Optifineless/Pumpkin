//! DecoratedPotBlock.onProjectileHit, using the shared destruction admission and loot path.

use std::sync::Arc;

use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::DecoratedPotLikeProperties,
    data_component_impl::CanBreakImpl,
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
    block::entities::BlockEntity,
    entity::{
        EntityBase,
        projectile::{
            arrow::ArrowEntity, egg::EggEntity, fireball::FireballEntity,
            firework_rocket::FireworkRocketEntity, trident::TridentEntity,
        },
    },
};

pub(super) struct DecoratedPotBreak<'a> {
    expected: BlockStateId,
    entity: Option<Arc<dyn BlockEntity>>,
    projectile: &'a dyn EntityBase,
}

impl World {
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
        let preparation = DecoratedPotBreak {
            expected,
            entity: self.get_block_entity(position),
            projectile,
        };
        let _ = self.break_block_with_limit_prepared(
            position,
            None,
            BlockFlags::NOTIFY_ALL,
            UPDATE_LIMIT,
            Some(&preparation),
        );
    }
}

impl DecoratedPotBreak<'_> {
    /// Claim removal after break admission, before any drops or callbacks can re-enter.
    pub(super) fn break_admitted(
        &self,
        world: &Arc<World>,
        position: &BlockPos,
        drop_items: bool,
        flags: BlockFlags,
        update_limit: u32,
    ) -> Option<BlockStateId> {
        if !may_break(world, position, self.projectile) {
            return None;
        }
        let replacement =
            crate::block::blocks::growing_plant::fluid_state_after_break(self.expected);
        let retired = self.claim_removal(world, position, replacement)?;
        if let Some(entity) = &retired.live {
            super::container_lifecycle::retire_block_entity(entity);
        }
        let mut properties = DecoratedPotLikeProperties::from_state_id(self.expected);
        properties.cracked = true;
        let cracked = properties.to_state_id(&Block::DECORATED_POT);
        // DecoratedPotBlock.onProjectileHit/getDrops: cracked loot uses the claimed entity.
        if drop_items {
            Self::drop_loot(world, position, cracked, retired.live.as_deref());
            crate::block::drop_experience(world, &Block::DECORATED_POT, position, None);
        }
        let effects = RetiredBlockEffects::capture(world, position, self.expected);
        retired.on_block_replaced(world, position);
        world.on_block_state_set_with_block_entity(
            position,
            cracked,
            replacement,
            flags - BlockFlags::SKIP_DROPS,
            update_limit,
            Some(&effects),
        );
        let java = CWorldEvent::new(
            WorldEvent::ParticlesDestroyBlock as i32,
            *position,
            cracked.as_u16().into(),
            false,
        );
        let bedrock = CLevelEvent {
            event_id: VarInt(LevelEvent::ParticlesDestroyBlock as i32),
            position: position.to_centered_f64().to_f32_lossy(),
            data: VarInt(BlockState::to_be_network_id(cracked) as i32),
        };
        world.broadcast_to_chunk_editioned(position.chunk_position(), &java, &bedrock);
        Some(cracked)
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
        if !same_entity(self.entity.as_ref(), current.as_ref()) {
            return None;
        }
        // The publication gate prevents a same-state replacement from changing the entity
        // between this identity check, the terrain CAS, and retirement of its saved/live caches.
        world.write_block_state_if(position, replacement, |state| state == self.expected)?;
        Some(world.remove_loaded_block_entity_with_gate(position, &mut loads))
    }

    fn drop_loot(
        world: &Arc<World>,
        position: &BlockPos,
        cracked: BlockStateId,
        entity: Option<&dyn BlockEntity>,
    ) {
        if let Some(table) = world.get_loot_table("minecraft:blocks/decorated_pot") {
            let params = LootContextParameters {
                world: Some(world.clone()),
                registry: world
                    .server
                    .upgrade()
                    .map(|server| server.datapack_manager.clone()),
                block_state: Some(cracked.to_state()),
                position: Some(position.to_f64()),
                killed_by_player: Some(false),
                block_entity_components: entity.map_or_else(
                    Default::default,
                    collect_block_entity_components_from_entity,
                ),
                ..Default::default()
            };
            for stack in crate::block::block_drops_with_context(world, position, &table, &params) {
                world.drop_stack(position, stack);
            }
        }
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
