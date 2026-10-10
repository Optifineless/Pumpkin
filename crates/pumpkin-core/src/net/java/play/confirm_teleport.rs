#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_confirm_teleport(&self, player: &Player, confirm_teleport: &SConfirmTeleport) {
        // Mirrors ServerGamePacketListenerImpl.handleAcceptTeleportPacket: ignore noncurrent IDs.
        let mut awaiting_teleport = player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((id, position)) = awaiting_teleport.as_ref()
            && id == &confirm_teleport.teleport_id
        {
            if confirm_teleport.position.x.is_nan()
                || confirm_teleport.position.y.is_nan()
                || confirm_teleport.position.z.is_nan()
                || !confirm_teleport.yaw.is_finite()
                || !confirm_teleport.pitch.is_finite()
            {
                self.try_kick(&TextComponent::translate_cross(
                    translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                    translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                    [],
                ));
                return;
            }

            // Keep the lock through the position update so a newer teleport cannot overtake it.
            player.get_entity().set_pos(*position);
            *awaiting_teleport = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer,
    };

    fn confirmation(teleport_id: i32) -> SConfirmTeleport {
        SConfirmTeleport {
            teleport_id: VarInt::from(teleport_id),
            position: Vector3::new(0.0, 64.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirmations_after_back_to_back_teleports_apply_the_newest_position() {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let first_position = Vector3::new(100.0, 64.0, 0.0);
        let second_position = Vector3::new(200.0, 64.0, 0.0);

        player.client().force_tp(&player.player, first_position);
        player.client().force_tp(&player.player, second_position);

        player
            .client()
            .handle_confirm_teleport(&player.player, &confirmation(1));
        assert!(!player.client().is_closed());
        assert_eq!(
            *player
                .player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some((VarInt::from(2), second_position))
        );

        player
            .client()
            .handle_confirm_teleport(&player.player, &confirmation(2));

        assert!(!player.client().is_closed());
        assert_eq!(player.player.get_entity().pos.load(), second_position);
        assert!(
            player
                .player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
        fixture.server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_confirmation_does_not_kick_or_clear_newer_teleport() {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let newer_position = Vector3::new(200.0, 64.0, 0.0);
        player.client().force_tp(&player.player, newer_position);

        player
            .client()
            .handle_confirm_teleport(&player.player, &confirmation(0));

        assert!(!player.client().is_closed());
        assert_eq!(
            *player
                .player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some((VarInt::from(1), newer_position))
        );
        assert_ne!(player.player.get_entity().pos.load(), newer_position);
        fixture.server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirmation_without_pending_teleport_does_not_kick() {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        let player = TestPlayer::new(&world);

        player
            .client()
            .handle_confirm_teleport(&player.player, &confirmation(1));

        assert!(!player.client().is_closed());
        assert!(
            player
                .player
                .awaiting_teleport
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
        fixture.server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn matching_confirmation_with_invalid_values_kicks() {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        player
            .client()
            .force_tp(&player.player, Vector3::new(200.0, 64.0, 0.0));
        let invalid_confirmation = SConfirmTeleport {
            teleport_id: VarInt::from(1),
            position: Vector3::new(f64::NAN, 64.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
        };

        player
            .client()
            .handle_confirm_teleport(&player.player, &invalid_confirmation);

        assert!(player.client().is_closed());
        fixture.server.shutdown().await;
    }
}
