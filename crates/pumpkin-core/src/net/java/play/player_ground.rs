#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_player_ground(&self, player: &Player, ground: &SSetPlayerGround) {
        if !player.has_client_loaded() {
            return;
        }
        let admitted = !player.get_entity().has_vehicle()
            && !player.is_movement_locked.load(Ordering::Relaxed)
            && player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none();
        if admitted {
            player.known_movement.record(Vector3::new(0.0, 0.0, 0.0));
        }
        player
            .living_entity
            .entity
            .on_ground
            .store(ground.on_ground, Ordering::Relaxed);
        if admitted {
            Self::check_player_fall(player, Vector3::new(0.0, 0.0, 0.0), ground.on_ground);
        }
    }
}
