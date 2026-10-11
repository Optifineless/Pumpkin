use std::sync::{
    Arc, Mutex,
    atomic::Ordering::{self, Relaxed},
};

use super::{CEntityPositionSync, Entity, EntityBase};
use crate::world::{
    World,
    portal::{PortalType, SourcePortalInfo},
};
use tokio_util::sync::CancellationToken;

pub mod passengers;
use passengers::PortalPassengers;

#[derive(Default)]
pub struct PortalTravelState {
    state: Mutex<PendingTravel>,
    #[cfg(test)]
    continuation_gate: Mutex<Option<ContinuationGate>>,
    #[cfg(test)]
    pub(crate) transfer_gate: Mutex<Option<ContinuationGate>>,
}

#[derive(Default)]
struct PendingTravel {
    generation: u64,
    pending: Option<CancellationToken>,
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
    as_passenger: bool,
    cancellation: CancellationToken,
}

impl PortalTravel {
    fn claim(entity: Arc<dyn EntityBase>, source: Arc<World>, as_passenger: bool) -> Option<Self> {
        // Entity.handlePortal / calculatePassengerTransition: one generation owns each arrival.
        let (generation, cancellation) = {
            let mut state = entity
                .get_entity()
                .portal_travel
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending.is_some() {
                return None;
            }
            state.generation = state.generation.wrapping_add(1);
            let cancellation = CancellationToken::new();
            state.pending = Some(cancellation.clone());
            (state.generation, cancellation)
        };
        Some(Self {
            life: entity
                .get_living_entity()
                .map(super::living::LivingEntity::damage_lifecycle),
            entity,
            source,
            generation,
            as_passenger,
            cancellation,
        })
    }

    /// Checks the captured trip after an await, before any arrival effects are applied.
    pub(crate) fn is_valid(&self) -> bool {
        self.is_current()
            && self
                .source
                .get_entity_by_id(self.entity.get_entity().entity_id)
                .is_some_and(|current| Arc::ptr_eq(&current, &self.entity))
    }

    /// Checks the initiating life/session even after source membership has been removed.
    pub(crate) fn is_current(&self) -> bool {
        self.is_current_in(&self.source)
    }

    pub(crate) fn is_current_in(&self, world: &Arc<World>) -> bool {
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
            && (self.as_passenger || !self.entity.is_passenger())
            && self.entity.get_living_entity().is_none_or(|living| {
                living.is_alive() && Some(living.damage_lifecycle()) == self.life
            })
            && !self.cancellation.is_cancelled()
            && Arc::ptr_eq(&entity.world.load_full(), world)
            && self
                .entity
                .get_player()
                .is_none_or(|player| !player.client.closed())
    }

    async fn teleport(
        self,
        portal_type: PortalType,
        destination_world: Arc<World>,
        source_portal: Option<SourcePortalInfo>,
    ) {
        if !self.is_valid() {
            return;
        }
        let Some(_source_admission) = self.source.admit_transfer() else {
            return;
        };
        let Some(_destination_admission) = destination_world.admit_transfer() else {
            return;
        };
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
            let destination = transition.new_world.clone();
            if let Some(player) = self.source.get_player_by_id(entity.entity_id) {
                // Keep the trip pending through the player's actual transfer, not a queued task.
                if player
                    .teleport_world_for_portal(
                        destination.clone(),
                        transition.position,
                        transition.yaw,
                        transition.pitch,
                        Some(&self),
                    )
                    .await
                    .is_none()
                {
                    return;
                }
                // Player publication moves passengers to the callback-selected transition.
                return;
            }
            Entity::teleport_portal_entity(
                &self.entity,
                transition.position,
                transition.yaw,
                transition.pitch,
                &destination,
            );
        }
    }
}

impl Drop for PortalTravel {
    fn drop(&mut self) {
        let mut state = self
            .entity
            .get_entity()
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.generation == self.generation {
            state.pending = None;
        }
    }
}

impl Entity {
    #[cfg(test)]
    pub(crate) fn portal_travel_pending_for_test(&self) -> bool {
        self.portal_travel.state.lock().unwrap().pending.is_some()
    }

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
        if let Some(pending) = state.pending.take() {
            pending.cancel();
        }
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
        let Some(trip) = PortalTravel::claim(entity, world, false) else {
            return;
        };
        let cancellation = trip.cancellation.clone();

        // Entity.handlePortal sets cooldown before resolving even a null destination.
        self.portal_cooldown
            .store(self.default_portal_cooldown(), Relaxed);

        // Entity.handlePortal / teleport: resolve the exit before moving the entity and passengers.
        // Chunk decoding uses the tick's Rayon pool, so chunk waits must yield on Tokio.
        // Logout drains the connection tracker before saving and removing this session.
        if let Some(player) = trip.source.get_player_by_id(self.entity_id) {
            let client = player.client.clone();
            player.client.spawn_task(async move {
                tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {},
                    () = client.await_close_interrupt() => {},
                    () = trip.teleport(portal_type, destination_world, source_portal) => {},
                }
            });
        } else {
            server.spawn_task(async move {
                tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {},
                    () = trip.teleport(portal_type, destination_world, source_portal) => {},
                }
            });
        }
    }

    // Entity.teleportCrossDimension -> ServerLevel.addDuringTeleport. Like our projectile
    // transfer, preserve Pumpkin's existing Arc rather than cloning its native entity state.
    fn teleport_portal_entity(
        entity: &Arc<dyn EntityBase>,
        position: pumpkin_util::math::vector3::Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        destination: &Arc<World>,
    ) -> bool {
        let base = entity.get_entity();
        let source = base.world.load_full();
        let generation = base
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation;
        let is_current = || {
            base.portal_travel
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .generation
                == generation
                && !base.is_removed()
                && base.is_alive()
                && Arc::ptr_eq(&base.world.load_full(), destination)
        };
        if base.is_removed() || !base.is_alive() {
            return false;
        }
        if Arc::ptr_eq(&source, destination) {
            let previous_rotation = (base.yaw.load(), base.pitch.load());
            base.apply_portal_teleport(position, yaw, pitch, destination);
            if is_current() {
                Self::teleport_portal_passengers(
                    base,
                    position,
                    yaw.map(|yaw| yaw - previous_rotation.0),
                    pitch.map(|pitch| pitch - previous_rotation.1),
                    destination,
                );
            }
            return is_current();
        }
        let old_position = base.pos.load();
        let old_rotation = (base.yaw.load(), base.pitch.load());
        let passengers = PortalPassengers::new(entity.clone());
        if !passengers.is_current_in(&source) || passengers.has_new_riders() {
            return false;
        }
        let source_state = source.spawn_state.load_full();
        let source_member = source
            .entities
            .load()
            .iter()
            .any(|other| Arc::ptr_eq(other, entity));
        // NaturalSpawner accounts against the source biome and chunk before they change.
        if source_member {
            source_state.remove_entity(&source, entity.as_ref());
        }
        base.set_pos(position);
        base.set_rotation(
            yaw.unwrap_or(old_rotation.0),
            pitch.unwrap_or(old_rotation.1),
        );
        base.set_portal_world(destination.clone());
        if !destination.insert_entity_batch(std::slice::from_ref(entity)) {
            base.set_pos(old_position);
            base.set_portal_world(source.clone());
            base.set_rotation(old_rotation.0, old_rotation.1);
            if source_member {
                source_state.add_entity(&source, entity.as_ref());
            }
            return false;
        }
        source
            .entity_tracker
            .remove_entity(entity.as_ref(), &source);
        source.entities.rcu(|entities| {
            entities
                .iter()
                .filter(|other| !Arc::ptr_eq(other, entity))
                .cloned()
                .collect::<Vec<_>>()
        });
        // Pairing/removal packets can re-enter native plugins after membership commits.
        if !is_current() {
            return false;
        }
        base.apply_portal_teleport(position, yaw, pitch, destination);
        if is_current() {
            passengers.teleport(
                position,
                yaw.map(|yaw| yaw - old_rotation.0),
                pitch.map(|pitch| pitch - old_rotation.1),
                destination,
            );
        }
        is_current()
    }

    /// Entity.teleportSetPosition and observer synchronization without superseding this trip.
    pub(super) fn apply_portal_teleport(
        &self,
        position: pumpkin_util::math::vector3::Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        world: &World,
    ) {
        // Update server-side position and bounding box
        self.set_pos(position);
        if let Some(yaw) = yaw {
            self.yaw.store(yaw);
        }
        if let Some(pitch) = pitch {
            self.set_pitch(pitch);
        }
        // Update cache so we don't send rubberbanding deltas
        self.last_sent_pos.store(position);
        // TODO: use `pumpkin_util::math::pack_degrees`.
        if let Some(yaw) = yaw {
            self.last_sent_yaw
                .store((yaw * 256.0 / 360.0).rem_euclid(256.0) as u8, Relaxed);
            self.last_sent_head_yaw
                .store((yaw * 256.0 / 360.0).rem_euclid(256.0) as u8, Relaxed);
        }
        if let Some(pitch) = pitch {
            self.last_sent_pitch
                .store((pitch * 256.0 / 360.0).rem_euclid(256.0) as u8, Relaxed);
        }
        let chunk_pos = self.chunk_pos.load();
        world.broadcast_to_chunk(
            chunk_pos,
            &CEntityPositionSync::new(
                self.entity_id.into(),
                position,
                self.velocity.load(),
                yaw.unwrap_or(self.yaw.load()),
                pitch.unwrap_or(self.pitch.load()),
                self.on_ground.load(Ordering::SeqCst),
            ),
        );
    }

    /// ServerPlayer.setServerLevel during the trip's own publication preserves its identity.
    pub(crate) fn set_portal_world(&self, world: Arc<World>) {
        let block_pos = self.block_pos.load();
        let biome = world.level.get_rough_biome(&block_pos);
        self.current_biome.store(Arc::new(biome));
        self.last_biome_update_pos.store(block_pos);
        self.world.store(world);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub mod review_tests;

#[cfg(test)]
mod final_tests;

#[cfg(test)]
mod rider_tests;

#[cfg(test)]
mod publication_tests;
