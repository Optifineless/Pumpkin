use super::World;
use crate::entity::Entity;
use pumpkin_util::math::vector3::Vector3;

impl World {
    /// Emits `GameEvent.ITEM_INTERACT_FINISH` with its entity context.
    /// This boundary currently fires only `GenericGameEvent` for plugins; Pumpkin has no
    /// vanilla vibration dispatcher. A future sculk dispatcher must use the source entity here.
    pub fn game_event_item_interact_finish(&self, entity: &Entity) {
        self.emit_game_event("item_interact_finish", entity.pos.load());
    }

    /// Emits `GameEvent.TELEPORT` at the departure position with its entity context.
    /// This boundary currently fires only the plugin event, with no sculk simulation.
    pub fn game_event_teleport(&self, _entity: &Entity, departure: Vector3<f64>) {
        self.emit_game_event("teleport", departure);
    }
}

impl World {
    /// Emits an event with the initiating entity's identity; legacy Wasm payloads remain unchanged.
    pub fn emit_game_event_with_source(
        &self,
        key: impl Into<String>,
        position: Vector3<f64>,
        source: Option<&Entity>,
    ) {
        // Level.gameEvent(sourceEntity, ...), including DoorBlock.setOpen's actor context.
        let mut event = crate::plugin::api::events::world::generic_game::GenericGameEvent::new(
            key.into(),
            position,
        );
        event.source_entity = source.map(|entity| entity.entity_uuid);
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }
}
