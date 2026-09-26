#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_protocol::java::server::play::SSpectatorAction;
use pumpkin_util::GameMode;
use pumpkin_data::attributes::Attributes;

impl JavaClient {
    pub fn handle_spectate_entity(&self, player: &Arc<Player>, packet: &SSpectatorAction) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();

        if player.gamemode.load() != GameMode::Spectator {
            return;
        }

        let Some(target_id) = packet.target_entity_id() else {
            return;
        };

        let world = player.world();
        if let Some(target) = world.get_entity_or_part(target_id) {
            // ServerGamePacketListenerImpl.handleSpectatorAction checks border, range, then pickability.
            let entity = target.get_entity();
            let block_pos = entity.block_pos.load().0;
            if !world.worldborder.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(f64::from(block_pos.x), f64::from(block_pos.z))
            { return; }
            let max_range = player.living_entity.get_attribute_value(&Attributes::ENTITY_INTERACTION_RANGE) + 3.0;
            if entity.bounding_box.load().squared_magnitude(player.eye_position()) >= max_range * max_range
                || entity.is_removed() || !target.can_hit()
            { return; }
            let target_pos = entity.pos.load();
            let target_yaw = target.get_entity().yaw.load();
            let target_pitch = target.get_entity().pitch.load();
            let target_id = target.get_entity().entity_id;

            player.set_camera_entity_id(target_id);
            let _ = player.request_teleport(target_pos, target_yaw, target_pitch);
        }
    }
}
