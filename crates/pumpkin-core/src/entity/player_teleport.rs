use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

use super::player::Player;
use crate::plugin::api::events::player::player_teleport::PlayerTeleportEvent;
use crate::{entity::EntityBase, world::World};
use pumpkin_data::damage::DamageType;
use pumpkin_protocol::java::client::play::CEntityPositionSync;

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
            if self.teleport_same_world(position, yaw, pitch).is_some()
                && current_world
                    .get_player_by_uuid(self.gameprofile.id)
                    .is_some_and(|player| std::ptr::eq(self, player.as_ref()))
            {
                self.finish_ender_pearl_teleport();
            }
            return;
        }

        let Some(player) = current_world.get_player_by_uuid(self.gameprofile.id) else {
            return;
        };
        let source_world = current_world;
        let task_player = player.clone();
        let _ = player.spawn_task(async move {
            task_player
                .teleport_world(world, position, Some(yaw), Some(pitch))
                .await;
            let arrived_world = task_player.world();
            if !Arc::ptr_eq(&source_world, &arrived_world)
                && arrived_world
                    .get_player_by_uuid(task_player.gameprofile.id)
                    .is_some_and(|player| Arc::ptr_eq(&player, &task_player))
            {
                task_player.finish_ender_pearl_teleport();
            }
        });
    }

    fn finish_ender_pearl_teleport(&self) {
        self.living_entity.reset_fall_distance_and_impulse_context();
        self.damage(self, 5.0, DamageType::ENDER_PEARL);
    }
}
