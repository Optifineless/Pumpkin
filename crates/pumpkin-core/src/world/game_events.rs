use super::World;
use crate::{
    entity::{Entity, living::LivingEntity},
    plugin::api::events::world::generic_game::GenericGameEvent,
};
use pumpkin_data::{BlockStateId, game_event::GameEvent};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use uuid::Uuid;

#[cfg(test)]
mod plugin_tests;
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

impl<'a> GameEventSource<'a> {
    fn capture(world: &'a World, entity: &'a Entity) -> Option<Self> {
        let current = world.get_entity_by_id(entity.entity_id)?;
        let snapshot = Self {
            entity,
            entity_id: entity.entity_id,
            uuid: entity.entity_uuid,
            world,
            position: entity.pos.load(),
            life: current
                .get_living_entity()
                .map(LivingEntity::damage_lifecycle),
        };
        drop(current);
        snapshot.is_current().then_some(snapshot)
    }

    fn is_current(&self) -> bool {
        if self.entity.is_removed() || !std::ptr::eq(self.entity.world.load().as_ref(), self.world)
        {
            return false;
        }
        let Some(current) = self.world.get_entity_by_id(self.entity_id) else {
            return false;
        };
        std::ptr::eq(current.get_entity(), self.entity)
            && current.get_entity().entity_uuid == self.uuid
            && current
                .get_living_entity()
                .map(LivingEntity::damage_lifecycle)
                == self.life
            && current.get_player().is_none_or(|player| {
                !player.client.closed() && self.world.contains_player_instance(player)
            })
    }
}

impl World {
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
        let source = context
            .source
            .and_then(|entity| GameEventSource::capture(self, entity));
        let mut event = GenericGameEvent::new(event_key.into(), position);
        let Some(server) = self.server.upgrade() else {
            return;
        };
        // ServerLevel.gameEvent -> GameEventDispatcher.post, after the existing plugin boundary.
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled || !server.advanced_config.sculk_game_events.enabled {
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
            || source.is_some_and(|source| !source.is_current())
        {
            return;
        }
        self.dispatch_game_event(EmittedGameEvent {
            event: resolved,
            position: event.position,
            source,
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
