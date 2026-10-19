//! DecoratedPotBlock.onProjectileHit, using the shared destruction admission and loot path.

use std::sync::Arc;

use pumpkin_data::{
    Block, BlockStateId,
    block_properties::DecoratedPotLikeProperties,
    data_component_impl::CanBreakImpl,
    tag::{self, Taggable},
};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::{GameMode, math::position::BlockPos};
use pumpkin_world::world::BlockFlags;

use super::{World, neighbor_updater::UPDATE_LIMIT};
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
    /// Revalidate after the break event; cancellation never changes the pot or its contents.
    pub(super) fn prepare(&self, world: &Arc<World>, position: &BlockPos) -> Option<()> {
        if world.get_block_state_id(position) != self.expected
            || !same_entity(
                self.entity.as_ref(),
                world.get_block_entity(position).as_ref(),
            )
            || !may_break(world, position, self.projectile)
        {
            return None;
        }
        let mut properties = DecoratedPotLikeProperties::from_state_id(self.expected);
        properties.cracked = true;
        let cracked = properties.to_state_id(&Block::DECORATED_POT);
        // Vanilla setBlock(..., 260): UPDATE_KNOWN_SHAPE | NOTIFY_LISTENERS.
        world.set_block_state(
            position,
            cracked,
            BlockFlags::UPDATE_KNOWN_SHAPE | BlockFlags::NOTIFY_LISTENERS,
        );
        (world.get_block_state_id(position) == cracked
            && same_entity(
                self.entity.as_ref(),
                world.get_block_entity(position).as_ref(),
            ))
        .then_some(())
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
