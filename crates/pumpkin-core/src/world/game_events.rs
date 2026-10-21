use super::World;
use super::game_event_sources::SourceMembership;
use crate::{
    entity::{Entity, EntityBase, living::LivingEntity, player::Player},
    plugin::api::events::world::generic_game::GenericGameEvent,
};
use pumpkin_data::{BlockStateId, game_event::GameEvent};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::Arc;
use uuid::Uuid;

#[cfg(test)]
mod actorless_tests;
#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod indexed_tests;
#[cfg(test)]
mod lifecycle_boundaries;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod native_reentry_tests;
#[cfg(test)]
mod plugin_tests;
#[cfg(test)]
mod projectile_controls;
#[cfg(test)]
mod projectile_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wrapper_tests;

/// Vanilla GameEvent.Context; the plugin API can edit the event, not this context.
#[derive(Default, Clone, Copy)]
pub struct GameEventContext<'a> {
    pub source: Option<&'a Entity>,
    pub affected_state: Option<BlockStateId>,
}

#[derive(Clone, Copy)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Consumed by future vibration listeners")
)]
pub struct GameEventSource<'a> {
    pub entity: &'a Entity,
    pub entity_id: i32,
    pub uuid: Uuid,
    pub world: &'a World,
    pub position: Vector3<f64>,
    pub life: Option<u64>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Consumed by future vibration listeners")
)]
#[derive(Clone, Copy)]
pub struct EmittedGameEvent<'a> {
    pub event: GameEvent,
    pub position: Vector3<f64>,
    pub source: Option<GameEventSource<'a>>,
    pub affected_state: Option<BlockStateId>,
}

pub struct CapturedSource<'a> {
    snapshot: GameEventSource<'a>,
    membership: Arc<SourceMembership>,
}

// Resolve every unsealed native getter before the final membership/scalar checks.
// None of these references retains the index owner or invokes virtual code on access.
struct SourceParts<'a> {
    entity: &'a Entity,
    living: Option<&'a LivingEntity>,
    player: Option<&'a Player>,
}

impl<'a> SourceParts<'a> {
    fn resolve(current: &'a dyn EntityBase) -> Self {
        Self {
            entity: current.get_entity(),
            living: current.get_living_entity(),
            player: current.get_player(),
        }
    }
}

impl<'a> CapturedSource<'a> {
    fn capture(world: &'a World, entity: &'a Entity) -> Option<Self> {
        let membership = world.game_event_source_membership(entity.entity_id)?;
        let current = membership.entity()?;
        let parts = SourceParts::resolve(current.as_ref());
        let snapshot = GameEventSource {
            entity,
            entity_id: entity.entity_id,
            uuid: entity.entity_uuid,
            world,
            position: entity.pos.load(),
            life: parts.living.map(LivingEntity::damage_lifecycle),
        };
        let captured = Self {
            snapshot,
            membership,
        };
        captured.matches(&parts).then_some(captured)
    }

    fn is_current(&self) -> bool {
        let source = &self.snapshot;
        let Some(current) = self.membership.entity() else {
            return false;
        };
        let parts = SourceParts::resolve(current.as_ref());
        let Some(membership) = source.world.game_event_source_membership(source.entity_id) else {
            return false;
        };
        #[cfg(test)]
        source
            .world
            .game_event_sources
            .comparisons
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Arc::ptr_eq(&membership, &self.membership) && self.matches(&parts)
    }

    fn matches(&self, current: &SourceParts<'_>) -> bool {
        let source = &self.snapshot;
        #[cfg(test)]
        source
            .world
            .game_event_sources
            .comparisons
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::ptr::eq(current.entity, source.entity)
            && current.entity.entity_id == source.entity_id
            && current.entity.entity_uuid == source.uuid
            && !source.entity.is_removed()
            && std::ptr::eq(source.entity.world.load().as_ref(), source.world)
            && current.living.map(LivingEntity::damage_lifecycle) == source.life
            && current.player.is_none_or(|player| !player.client.closed())
    }
}

impl World {
    pub(crate) fn emit_game_event_from_action(
        &self,
        event_key: impl Into<String>,
        position: Vector3<f64>,
        source: Option<&Entity>,
        action: Option<&CapturedSource<'_>>,
    ) {
        if source.is_some() {
            self.emit_game_event_after_action(
                event_key,
                position,
                || GameEventContext {
                    source,
                    affected_state: None,
                },
                action,
            );
        } else {
            self.emit_game_event(event_key, position);
        }
    }

    pub(crate) fn capture_game_event_source_for_action<'a>(
        &'a self,
        entity: &'a Entity,
    ) -> Option<CapturedSource<'a>> {
        self.game_event_forwarding_enabled()
            .then(|| CapturedSource::capture(self, entity))
            .flatten()
    }

    pub(crate) fn emit_game_event_after_action<'a>(
        &self,
        event_key: impl Into<String>,
        position: Vector3<f64>,
        context: impl FnOnce() -> GameEventContext<'a>,
        action: Option<&CapturedSource<'_>>,
    ) {
        self.emit_game_event_with_validation(event_key, position, context, || {
            action.is_some_and(CapturedSource::is_current)
        });
    }

    pub(crate) fn game_event_forwarding_enabled(&self) -> bool {
        self.server
            .upgrade()
            .is_some_and(|server| server.advanced_config.sculk_game_events.enabled)
    }

    pub(crate) fn emit_game_event_from(
        &self,
        event_key: impl Into<String>,
        position: Vector3<f64>,
        source: Option<&Entity>,
    ) {
        self.emit_game_event_with_context(
            event_key,
            position,
            GameEventContext {
                source,
                affected_state: None,
            },
        );
    }

    /// Emits `GameEvent.ITEM_INTERACT_FINISH` at the user's existing event position.
    pub fn game_event_item_interact_finish(&self, entity: &Entity) {
        self.emit_game_event_with_context(
            "item_interact_finish",
            entity.pos.load(),
            GameEventContext {
                source: Some(entity),
                affected_state: None,
            },
        );
    }

    /// Emits `GameEvent.TELEPORT` at the successful teleport's departure position.
    pub fn game_event_teleport(&self, entity: &Entity, departure: Vector3<f64>) {
        self.emit_game_event_with_context(
            "teleport",
            departure,
            GameEventContext {
                source: Some(entity),
                affected_state: None,
            },
        );
    }

    pub(crate) fn emit_game_event_with_context(
        &self,
        event_key: impl Into<String>,
        position: Vector3<f64>,
        context: GameEventContext<'_>,
    ) {
        self.emit_game_event_with_lazy_context(event_key, position, || context);
    }

    pub(crate) fn emit_game_event_with_lazy_context<'a>(
        &self,
        event_key: impl Into<String>,
        position: Vector3<f64>,
        context: impl FnOnce() -> GameEventContext<'a>,
    ) {
        self.emit_game_event_with_validation(event_key, position, context, || true);
    }

    fn emit_game_event_with_validation<'a>(
        &self,
        event_key: impl Into<String>,
        position: Vector3<f64>,
        context: impl FnOnce() -> GameEventContext<'a>,
        validate_action: impl FnOnce() -> bool,
    ) {
        let Some(server) = self.server.upgrade() else {
            return;
        };
        let enabled = server.advanced_config.sculk_game_events.enabled;
        let context = if enabled {
            context()
        } else {
            GameEventContext::default()
        };
        let source = context
            .source
            .and_then(|entity| CapturedSource::capture(self, entity));
        let mut event = GenericGameEvent::new(event_key.into(), position);
        // ServerLevel.gameEvent -> GameEventDispatcher.post, after the existing plugin boundary.
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled || !enabled {
            return;
        }
        let Some(resolved) = GameEvent::from_name(&event.event_key) else {
            return;
        };
        if !event.position.x.is_finite()
            || !event.position.y.is_finite()
            || !event.position.z.is_finite()
            || !Self::is_valid(BlockPos::floored_v(event.position))
            || (context.source.is_some() && source.is_none())
            || source.as_ref().is_some_and(|source| !source.is_current())
            || !validate_action()
        {
            return;
        }
        self.dispatch_game_event(EmittedGameEvent {
            event: resolved,
            position: event.position,
            source: source.map(|source| source.snapshot),
            affected_state: context.affected_state,
        });
    }

    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "The test sink consumes the world until vibration listeners land"
        )
    )]
    fn dispatch_game_event(&self, event: EmittedGameEvent<'_>) {
        pumpkin_util::sync::assert_not_in_snapshot(
            pumpkin_util::sync::SnapshotBoundary::Event,
            "World::dispatch_game_event",
        );
        pumpkin_util::sync::assert_no_guards_held("World::dispatch_game_event");
        // Spatial listener admission and delivery belong to the next foundation slice.
        #[cfg(test)]
        test_support::observe(self, &event);
        #[cfg(not(test))]
        let _ = event;
    }
}
