use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

use super::player::Player;
use crate::plugin::api::events::player::player_teleport::PlayerTeleportEvent;
use crate::{entity::EntityBase, world::World};
use pumpkin_data::damage::DamageType;
use pumpkin_protocol::java::client::play::CEntityPositionSync;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TeleportWorldOutcome {
    NotCommitted,
    Committed,
}

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

    /// Follows ServerPlayer.teleport's same-world return point and reports Pumpkin event cancellation.
    pub(super) fn teleport_same_world(
        &self,
        position: Vector3<f64>,
        yaw: f32,
        pitch: f32,
    ) -> Option<Vector3<f64>> {
        let position = self.request_teleport(position, yaw, pitch)?;
        let entity = self.get_entity();
        let chunk_pos = entity.chunk_pos.load();
        entity.world.load().broadcast_to_chunk_except(
            chunk_pos,
            &[entity.entity_uuid],
            &CEntityPositionSync::new(
                entity.entity_id.into(),
                position,
                Vector3::new(0.0, 0.0, 0.0),
                yaw,
                pitch,
                entity.on_ground.load(std::sync::atomic::Ordering::SeqCst),
            ),
        );
        Some(position)
    }

    /// Teleports a pearl's player owner and applies pearl effects only after success.
    pub(crate) fn teleport_for_ender_pearl(&self, position: Vector3<f64>, world: Arc<World>) {
        let current_world = self.world();
        let yaw = self.get_entity().yaw.load();
        let pitch = self.get_entity().pitch.load();
        if Arc::ptr_eq(&current_world, &world) {
            if let Some(accepted_position) = self.teleport_same_world(position, yaw, pitch)
                && current_world
                    .get_player_by_uuid(self.gameprofile.id)
                    .is_some_and(|player| std::ptr::eq(self, player.as_ref()))
            {
                self.finish_ender_pearl_teleport();
                current_world.play_sound(
                    pumpkin_data::sound::Sound::EntityPlayerTeleport,
                    pumpkin_data::sound::SoundCategory::Players,
                    &accepted_position,
                );
            }
            return;
        }

        let Some(player) = current_world.get_player_by_uuid(self.gameprofile.id) else {
            return;
        };
        let task_player = player.clone();
        let expected_player = task_player.clone();
        let life = task_player.living_entity.damage_lifecycle();
        player.spawn_task(async move {
            let _ = task_player
                .teleport_world_with_commit(
                    world,
                    position,
                    Some(yaw),
                    Some(pitch),
                    move |player, was_loaded| {
                        // ThrownEnderpearl.onHit applies these after ServerPlayer.teleport succeeds.
                        if Arc::ptr_eq(player, &expected_player) {
                            player.finish_ender_pearl_teleport_for_lifecycle(life, was_loaded);
                        }
                    },
                    move |published_world, accepted_position| {
                        // ThrownEnderpearl.onHit sounds after damage and the destination position packet.
                        published_world.play_sound(
                            pumpkin_data::sound::Sound::EntityPlayerTeleport,
                            pumpkin_data::sound::SoundCategory::Players,
                            &accepted_position,
                        );
                    },
                )
                .await;
        });
    }

    fn finish_ender_pearl_teleport(&self) {
        self.living_entity.reset_fall_distance_and_impulse_context();
        self.damage(self, 5.0, DamageType::ENDER_PEARL);
    }

    fn finish_ender_pearl_teleport_for_lifecycle(&self, lifecycle: u64, was_loaded: bool) {
        self.living_entity
            .hurt_ender_pearl_for_lifecycle(self, lifecycle, was_loaded);
    }
}
