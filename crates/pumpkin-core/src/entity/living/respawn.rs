use super::LivingEntity;
use crate::{
    entity::{EntityBase, RemovalReason, player::Player},
    world::World,
};
use pumpkin_util::math::vector3::Vector3;
use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering::Relaxed},
};
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct WorldTransferState {
    active: AtomicBool,
    finished: Notify,
    #[cfg(test)]
    waiting: std::sync::Mutex<Option<Arc<Notify>>>,
}

pub struct WorldTransfer<'a>(&'a LivingEntity);

impl Drop for WorldTransfer<'_> {
    fn drop(&mut self) {
        let _owner = self.0.own_damage();
        self.0.world_transfer.active.store(false, Relaxed);
        self.0.world_transfer.finished.notify_waiters();
    }
}

/// Captures a player life under ownership so callbacks cannot continue an old tick.
pub struct PlayerTickLife<'a>(Option<(&'a LivingEntity, u64)>);

impl<'a> PlayerTickLife<'a> {
    pub(crate) fn capture(caller: &'a dyn EntityBase) -> Self {
        Self(caller.get_player().map(|player| {
            let living = &player.living_entity;
            (living, living.damage_lifecycle())
        }))
    }

    pub(crate) fn is_current(&self) -> bool {
        // PlayerList.respawn replaces ServerPlayer; Pumpkin reuses it and invalidates old frames.
        self.0.is_none_or(|(living, lifecycle)| {
            debug_assert!(living.damage_owner.is_owned_by_current_thread());
            !living.is_respawning() && living.damage_lifecycle() == lifecycle
        })
    }
}

impl LivingEntity {
    pub(crate) fn is_respawning(&self) -> bool {
        self.respawning.load(Relaxed)
    }

    /// Detaches the current world, refusing a transfer, an existing respawn, or a removed life.
    // PlayerList.respawn removes the old life before restoreFrom and publishes after snapTo.
    pub(crate) fn begin_respawn(&self) -> Option<Arc<World>> {
        let _owner = self.own_damage();
        if self.world_transfer.active.load(Relaxed)
            || self.is_respawning()
            || self.entity.is_removed()
        {
            return None;
        }
        let source = self.entity.world.load_full();
        self.respawning.store(true, Relaxed);
        self.damage_owner.reset();
        source.players.rcu(|players| {
            players
                .iter()
                .filter(|p| p.entity_id() != self.entity.entity_id)
                .cloned()
                .collect::<Vec<_>>()
        });
        Some(source)
    }

    /// Detaches the live source world once any committed transfer has finished.
    pub(crate) async fn begin_respawn_after_transfer(&self, player: &Player) -> Option<Arc<World>> {
        loop {
            let finished = self.world_transfer.finished.notified();
            {
                let _owner = self.own_damage();
                if player.client.closed() || self.entity.is_removed() || self.is_respawning() {
                    return None;
                }
                if !self.world_transfer.active.load(Relaxed) {
                    return self.begin_respawn();
                }
            }
            #[cfg(test)]
            if let Some(waiting) = self
                .world_transfer
                .waiting
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                waiting.notify_one();
            }
            finished.await;
        }
    }

    /// Signals once when respawn reaches the active-transfer wait.
    #[cfg(test)]
    pub(crate) fn notify_when_respawn_waits(&self) -> Arc<Notify> {
        let waiting = Arc::new(Notify::new());
        *self
            .world_transfer
            .waiting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(waiting.clone());
        waiting
    }

    /// Reserves a transfer from this source until the returned guard is dropped.
    // ServerPlayer.teleport and PlayerList.respawn cannot interleave on the vanilla main thread.
    pub(crate) fn begin_world_transfer(&self, source: &World) -> Option<WorldTransfer<'_>> {
        let _owner = self.own_damage();
        if self.is_respawning()
            || self.entity.is_removed()
            || self.entity.world.load().uuid != source.uuid
            || self.world_transfer.active.swap(true, Relaxed)
        {
            return None;
        }
        Some(WorldTransfer(self))
    }

    pub(crate) fn abort_respawn(&self) -> bool {
        let _owner = self.own_damage();
        if !self.is_respawning() || self.entity.removed.swap(true, Relaxed) {
            return false;
        }
        // PlayerList.remove -> removePlayerImmediately(UNLOADED_WITH_PLAYER) is permanent.
        self.entity
            .removal_reason
            .store(Some(RemovalReason::UnloadedWithPlayer));
        self.damage_owner.reset();
        true
    }

    pub(crate) fn respawn_can_continue(&self, player: &Player) -> bool {
        self.is_respawning()
            && !self.entity.removed.load(Relaxed)
            && !self.entity.is_removed()
            && !player.client.closed()
    }

    pub(crate) fn publish_respawn(
        &self,
        player: &Arc<Player>,
        world: &Arc<World>,
        previous_world: &World,
        position: Vector3<f64>,
        yaw: f32,
        pitch: f32,
    ) -> bool {
        let _owner = self.own_damage();
        if !self.respawn_can_continue(player) || player.world().uuid != world.uuid {
            return false;
        }
        // PlayerList.respawn snaps the restored life before addRespawnedPlayer.
        self.entity.set_pos(position);
        self.entity.set_rotation(yaw, pitch);
        self.entity.last_pos.store(position);
        if world.uuid == previous_world.uuid {
            world
                .entity_tracker
                .respawn_entity(&(player.clone() as Arc<dyn EntityBase>), world);
        } else {
            world.add_arriving_player(player);
            world.entity_tracker.repair_respawned_player(player, world);
        }
        world.players.rcu(|players| {
            let mut players = (**players).clone();
            if !players
                .iter()
                .any(|p| p.entity_id() == self.entity.entity_id)
            {
                players.push(player.clone());
            }
            players
        });
        self.respawning.store(false, Relaxed);
        true
    }

    /// Publishes the restored life or disconnects and finishes its teardown.
    pub(crate) async fn complete_respawn(
        &self,
        player: &Arc<Player>,
        world: &Arc<World>,
        previous_world: &World,
        position: Vector3<f64>,
        yaw: f32,
        pitch: f32,
    ) -> bool {
        if self.publish_respawn(player, world, previous_world, position, yaw, pitch) {
            return true;
        }
        // PlayerList.remove permanently tears down a life that cannot be published.
        if previous_world.uuid != player.world().uuid {
            previous_world.remove_player(player, false).await;
            player.clean_up_chunk_tickets(&previous_world.level);
            player.unload_watched_chunks(previous_world).await;
        }
        player.client.try_kick(
            crate::net::DisconnectReason::UnrecoverableError,
            &pumpkin_util::text::TextComponent::text("Could not complete respawn"),
        );
        player.remove().await;
        let _owner = self.own_damage();
        self.respawning.store(false, Relaxed);
        false
    }

    /// Runs cached tick work only while its player life is still published.
    pub(crate) fn for_published_life(&self, lifecycle: u64, action: impl FnOnce()) {
        let _owner = self.own_damage();
        if !self.is_respawning() && self.damage_lifecycle() == lifecycle {
            action();
        }
    }
}
