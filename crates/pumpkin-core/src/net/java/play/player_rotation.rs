#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_rotation(&self, player: &Player, rotation: &SPlayerRotation) {
        let _life = player.living_entity.own_damage();
        if player.living_entity.is_respawning() || !player.has_client_loaded() {
            return;
        }
        if !rotation.yaw.is_finite() || !rotation.pitch.is_finite() {
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                [],
            ));
            return;
        }
        if !player.get_entity().has_vehicle()
            && !player.is_movement_locked.load(Ordering::Relaxed)
            && player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        {
            player.known_movement.record(Vector3::new(0.0, 0.0, 0.0));
        }
        let entity = &player.get_entity();
        entity.on_ground.store(rotation.ground, Ordering::Relaxed);
        entity.set_rotation(
            wrap_degrees(rotation.yaw) % 360.0,
            wrap_degrees(rotation.pitch),
        );
        // Send the new position to tracking players only.
        let entity_id = entity.entity_id;
        // TODO: use `pumpkin_util::math::pack_degrees`.
        let yaw = (entity.yaw.load() * 256.0 / 360.0).rem_euclid(256.0);
        let pitch = (entity.pitch.load() * 256.0 / 360.0).rem_euclid(256.0);

        let world = entity.world.load_full();
        let je_packet =
            CUpdateEntityRot::new(entity_id.into(), yaw as u8, pitch as u8, rotation.ground);

        let pos = entity.pos.load();

        // MODE_ROTATION not used for other players -> AvatarEntity always sends
        // MODE_NORMAL (client already lerps). MODE_ROTATION drops live head yaw on Bedrock.
        let be_packet =
            bedrock_move_player_packet(entity, pos, CMovePlayer::MODE_NORMAL, rotation.ground);

        world.send_to_tracking_players_editioned(entity, &je_packet, &be_packet);

        let je_packet = CHeadRot::new(entity_id.into(), yaw as u8);
        world.send_to_tracking_players(entity, &je_packet);
    }
}
