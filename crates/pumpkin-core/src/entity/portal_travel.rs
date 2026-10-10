use std::sync::{Arc, Mutex, atomic::Ordering::Relaxed};

use super::{Entity, EntityBase};
use crate::world::{
    World,
    portal::{PortalType, SourcePortalInfo},
};

#[derive(Default)]
pub struct PortalTravelState {
    state: Mutex<PendingTravel>,
    #[cfg(test)]
    continuation_gate: Mutex<Option<ContinuationGate>>,
}

#[derive(Default)]
struct PendingTravel {
    generation: u64,
    pending: bool,
}

#[cfg(test)]
type ContinuationGate = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

pub struct PortalTravel {
    entity: Arc<dyn EntityBase>,
    source: Arc<World>,
    generation: u64,
    life: Option<u64>,
}

impl PortalTravel {
    /// Checks the captured trip after an await, before any arrival effects are applied.
    pub(crate) fn is_valid(&self) -> bool {
        let entity = self.entity.get_entity();
        let generation = entity
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation;
        // Entity.canUsePortal / teleport reject dead or removed entities.
        generation == self.generation
            && !entity.removed.load(Relaxed)
            && entity.is_alive()
            && !self.entity.is_passenger()
            && self.entity.get_living_entity().is_none_or(|living| {
                living.is_alive() && Some(living.damage_lifecycle()) == self.life
            })
            && Arc::ptr_eq(&entity.world.load_full(), &self.source)
            && self
                .source
                .get_entity_by_id(entity.entity_id)
                .is_some_and(|current| Arc::ptr_eq(&current, &self.entity))
    }

    async fn teleport(
        self,
        portal_type: PortalType,
        destination_world: Arc<World>,
        source_portal: Option<SourcePortalInfo>,
        yaw: f32,
    ) {
        if !self.is_valid() {
            return;
        }
        let transition = portal_type
            .get_portal_destination(
                &self.source,
                destination_world,
                self.entity.as_ref(),
                source_portal.as_ref(),
            )
            .await;

        if let Some(transition) = transition.filter(|_| self.is_valid()) {
            let entity = self.entity.get_entity();
            let destination = transition.new_world;
            if let Some(player) = self.source.get_player_by_id(entity.entity_id) {
                // Keep the trip pending through the player's actual transfer, not a queued task.
                if !player
                    .teleport_world_for_portal(
                        destination.clone(),
                        transition.position,
                        transition.yaw,
                        transition.pitch,
                        Some(&self),
                    )
                    .await
                {
                    return;
                }
            } else {
                entity
                    .portal_cooldown
                    .store(entity.default_portal_cooldown(), Relaxed);
                self.entity.teleport(
                    transition.position,
                    transition.yaw,
                    transition.pitch,
                    destination.clone(),
                );
            }
            Entity::teleport_passengers_recursive(
                entity,
                transition.position,
                transition.yaw.map(|new_yaw| new_yaw - yaw),
                &destination,
            );
        }
    }
}

impl Drop for PortalTravel {
    fn drop(&mut self) {
        self.entity
            .get_entity()
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending = false;
    }
}

impl Entity {
    #[cfg(test)]
    pub(crate) async fn pause_portal_search_for_test(&self) {
        let gate = self
            .portal_travel
            .continuation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((ready, resume)) = gate {
            let _ = ready.send(());
            let _ = resume.await;
        }
    }

    /// Supersedes a pending portal trip when another teleport or life change starts.
    pub(crate) fn invalidate_portal_travel(&self) {
        let mut state = self
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
    }

    /// Queues destination loading and teleporting without waiting in the world tick.
    pub(super) fn teleport_through_portal(
        &self,
        portal_type: PortalType,
        destination_world: Arc<World>,
        source_portal: Option<SourcePortalInfo>,
    ) {
        let world = self.world.load_full();
        let Some(server) = world.server.upgrade() else {
            return;
        };
        let Some(entity) = world.get_entity_by_id(self.entity_id) else {
            return;
        };
        // Entity.handlePortal resolves synchronously in vanilla; allow only one yielded trip here.
        let generation = {
            let mut state = self
                .portal_travel
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending {
                return;
            }
            state.generation = state.generation.wrapping_add(1);
            state.pending = true;
            state.generation
        };
        let trip = PortalTravel {
            life: entity
                .get_living_entity()
                .map(super::living::LivingEntity::damage_lifecycle),
            entity,
            source: world,
            generation,
        };
        let yaw = self.yaw.load();

        // Entity.handlePortal / teleport: resolve the exit before moving the entity and passengers.
        // Chunk decoding uses the tick's Rayon pool, so chunk waits must yield on Tokio.
        server.spawn_task(trip.teleport(portal_type, destination_world, source_portal, yaw));
    }
}

#[cfg(test)]
mod tests;
