use std::sync::Arc;

use pumpkin_util::math::vector3::Vector3;

use super::Player;
use crate::{
    entity::{EntityBase, portal_travel::PortalTravel},
    world::{World, transfer_admission::WorldAdmission},
};

impl Player {
    pub(super) async fn prepare_world_transfer<'a>(
        &self,
        current_world: &World,
        new_world: &'a World,
        portal: Option<&PortalTravel>,
    ) -> Option<WorldAdmission<'a>> {
        let admission = new_world.admit_transfer()?;
        #[cfg(test)]
        if portal.is_some() {
            let gate = self
                .get_entity()
                .portal_travel
                .transfer_gate
                .lock()
                .unwrap()
                .take();
            if let Some((ready, resume)) = gate {
                let _ = ready.send(());
                let _ = resume.await;
            }
        }
        self.unload_watched_chunks(current_world).await;
        // ServerPlayer.teleport is synchronous with removal on vanilla's server thread.
        // Yielded watcher cleanup must not publish an obsolete life or closed session.
        if self.client.closed() || portal.is_some_and(|trip| !trip.is_valid()) {
            return None;
        }
        Some(admission)
    }

    pub(super) async fn commit_world_transfer(
        self: &Arc<Self>,
        current_world: &Arc<World>,
        new_world: &Arc<World>,
        position: Vector3<f64>,
        rotation: (f32, f32),
        portal: Option<&PortalTravel>,
    ) -> bool {
        // Packet delivery can yield too. Reject logout/life/world changes before source removal.
        if self.client.closed() || portal.is_some_and(|trip| !trip.is_valid()) {
            return false;
        }
        if current_world.remove_player(self, false).await.is_none() {
            return false;
        }
        self.publish_world_transfer(current_world, new_world, position, rotation, portal)
    }

    fn publish_world_transfer(
        self: &Arc<Self>,
        current_world: &Arc<World>,
        new_world: &Arc<World>,
        position: Vector3<f64>,
        rotation: (f32, f32),
        portal: Option<&PortalTravel>,
    ) -> bool {
        // Recheck after source removal's packet/plugin effects, immediately before publication.
        if self.client.closed() || portal.is_some_and(|trip| !trip.is_current()) {
            if !self.client.closed()
                && !self
                    .get_entity()
                    .removed
                    .load(std::sync::atomic::Ordering::Relaxed)
                && Arc::ptr_eq(&self.world(), current_world)
                && let Some(_source_admission) = current_world.admit_transfer()
            {
                current_world.players.rcu(|players| {
                    let mut players = (**players).clone();
                    if !players
                        .iter()
                        .any(|player| player.gameprofile.id == self.gameprofile.id)
                    {
                        players.push(self.clone());
                    }
                    players
                });
                current_world.add_arriving_player(self);
            }
            return false;
        }
        self.change_world_chunks(&current_world.level, new_world);
        let entity = self.get_entity();
        entity.set_world(new_world.clone());
        entity.set_pos(position);
        entity.set_rotation(rotation.0, rotation.1);
        entity.last_pos.store(position);
        new_world.players.rcu(|players| {
            let mut players = (**players).clone();
            players.push(self.clone());
            players
        });
        // ServerPlayer.teleport -> addDuringTeleport publishes after positioning, without awaits.
        new_world.add_arriving_player(self);
        true
    }
}
