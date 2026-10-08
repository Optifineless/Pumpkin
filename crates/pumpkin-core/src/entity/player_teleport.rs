use pumpkin_util::math::vector3::Vector3;

use super::player::Player;
use crate::plugin::api::events::player::player_teleport::PlayerTeleportEvent;

impl Player {
    /// Resolves the plugin-approved destination before any teleport side effects.
    pub(super) fn accept_teleport_destination(
        &self,
        requested: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        let world = self.world();
        let server = world.server.upgrade()?;
        let Some(player) = world.get_player_by_uuid(self.gameprofile.id) else {
            return Some(requested);
        };
        let mut event = PlayerTeleportEvent {
            player,
            from: self.position(),
            to: requested,
            cancelled: false,
        };
        server.plugin_manager.fire_blocking(&server, &mut event);
        (!event.cancelled).then_some(event.to)
    }
}
