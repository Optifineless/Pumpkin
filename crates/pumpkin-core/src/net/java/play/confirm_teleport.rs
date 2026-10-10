#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_confirm_teleport(&self, player: &Player, confirm_teleport: &SConfirmTeleport) {
        // Mirrors ServerGamePacketListenerImpl.handleAcceptTeleportPacket: ignore noncurrent IDs.
        let mut awaiting_teleport = player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The counter retains the latest ID after the pending position is cleared.
        let latest_id = awaiting_teleport.as_ref().map_or_else(
            || player.teleport_id_count.load(Ordering::Relaxed),
            |(id, _)| id.0,
        );
        if confirm_teleport.teleport_id.0 != latest_id {
            return;
        }

        if awaiting_teleport.is_none()
            || confirm_teleport.position.x.is_nan()
            || confirm_teleport.position.y.is_nan()
            || confirm_teleport.position.z.is_nan()
            || !confirm_teleport.yaw.is_finite()
            || !confirm_teleport.pitch.is_finite()
        {
            // PacketSentEvent may request another teleport while the disconnect is sent.
            drop(awaiting_teleport);
            self.try_kick(&TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                translation::java::MULTIPLAYER_DISCONNECT_INVALID_PLAYER_MOVEMENT,
                [],
            ));
            return;
        }

        if let Some((_, position)) = awaiting_teleport.as_ref() {
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
        entity::death_test_world::DeathTestWorld,
        net::java::combat_test_support::TestPlayer,
        plugin::{BoxFuture, EventHandler, EventPriority, server::packet::PacketSentEvent},
    };
    use pumpkin_data::packet::CURRENT_MC_VERSION;
    use pumpkin_protocol::{ClientPacket, MultiVersionJavaPacket, RawPacket};
    use pumpkin_util::version::JavaMinecraftVersion;
    use std::sync::atomic::AtomicBool;

    struct TeleportOnDisconnect {
        requested: AtomicBool,
    }

    impl EventHandler<PacketSentEvent> for TeleportOnDisconnect {
        fn handle_blocking<'a>(
            &'a self,
            _: &'a Arc<Server>,
            event: &'a mut PacketSentEvent,
        ) -> BoxFuture<'a, ()> {
            // Avoid hanging the test on regression, then exercise the real reentrant call.
            if let Ok(guard) = event.player.awaiting_teleport.try_lock() {
                drop(guard);
                self.requested.store(
                    event
                        .player
                        .request_teleport(Vector3::new(300.0, 64.0, 0.0), 0.0, 0.0)
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
            Box::pin(async {})
        }
    }

    fn register_disconnect_hook(
        fixture: &DeathTestWorld,
        player: &TestPlayer,
    ) -> Arc<TeleportOnDisconnect> {
        player.client().set_player(player.player.clone());
        // try_kick translates outbound packets for older clients through PacketSentEvent.
        player
            .client()
            .version
            .store(JavaMinecraftVersion::V_1_21_11);
        let hook = Arc::new(TeleportOnDisconnect {
            requested: AtomicBool::new(false),
        });
        fixture
            .server
            .plugin_manager
            .register(hook.clone(), EventPriority::Normal, true);
        hook
    }

    fn dispatch_confirmation(
        fixture: &DeathTestWorld,
        player: &TestPlayer,
        confirmation: &SConfirmTeleport,
    ) {
        let mut payload = Vec::new();
        assert!(
            confirmation
                .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
                .is_ok()
        );
        assert!(
            player
                .client()
                .handle_play_packet(
                    &player.player,
                    &fixture.server,
                    &RawPacket {
                        id: SConfirmTeleport::to_id(CURRENT_MC_VERSION),
                        payload: payload.into(),
                    },
                )
                .is_ok()
        );
    }

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
        let hook = register_disconnect_hook(&fixture, &player);

        dispatch_confirmation(&fixture, &player, &invalid_confirmation);

        assert!(player.client().is_closed());
        assert!(hook.requested.load(Ordering::Relaxed));
        fixture.server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn repeated_latest_confirmation_kicks_but_older_confirmation_is_ignored() {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let position = Vector3::new(200.0, 64.0, 0.0);
        player.client().force_tp(&player.player, position);
        dispatch_confirmation(&fixture, &player, &confirmation(1));
        assert!(!player.client().is_closed());
        assert_eq!(player.player.position(), position);

        dispatch_confirmation(&fixture, &player, &confirmation(0));
        assert!(!player.client().is_closed());
        let hook = register_disconnect_hook(&fixture, &player);
        dispatch_confirmation(&fixture, &player, &confirmation(1));
        assert!(player.client().is_closed());
        assert!(hook.requested.load(Ordering::Relaxed));
        fixture.server.shutdown().await;
    }
}
