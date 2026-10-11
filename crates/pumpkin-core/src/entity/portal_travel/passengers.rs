//! Entity.teleportCrossDimension ejects riders before publishing the destination vehicle.

use std::sync::{Arc, atomic::Ordering::Relaxed};

use pumpkin_util::math::vector3::Vector3;

use super::{Entity, EntityBase, PortalTravel};
use crate::world::World;

pub struct PortalPassengers {
    vehicle: Arc<dyn EntityBase>,
    source: Arc<World>,
    generation: u64,
    riders: Vec<PortalTravel>,
}

impl PortalPassengers {
    pub(crate) fn new(vehicle: Arc<dyn EntityBase>) -> Self {
        let entity = vehicle.get_entity();
        let source = entity.world.load_full();
        let generation = entity
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation;
        let passengers = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut riders = Vec::with_capacity(passengers.len());
        for passenger in &passengers {
            let base = passenger.get_entity();
            if base
                .get_vehicle()
                .is_some_and(|current| Arc::ptr_eq(&current, &vehicle))
            {
                base.invalidate_portal_travel();
                if let Some(trip) =
                    PortalTravel::claim(passenger.clone(), base.world.load_full(), true)
                {
                    riders.push(trip);
                }
                let mut current = base
                    .vehicle
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if current
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &vehicle))
                {
                    *current = None;
                }
            }
        }
        entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|current| !passengers.iter().any(|old| Arc::ptr_eq(current, old)));
        // Existing source observers must lose the riding list before any vehicle publication.
        // All generation/riding locks are released before native packet callbacks.
        if !passengers.is_empty() {
            Entity::publish_portal_passengers(&vehicle);
        }
        Self {
            vehicle,
            source,
            generation,
            riders,
        }
    }

    pub(crate) fn is_current_in(&self, world: &Arc<World>) -> bool {
        let entity = self.vehicle.get_entity();
        entity
            .portal_travel
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation
            == self.generation
            && !entity.is_removed()
            && entity.is_alive()
            && Arc::ptr_eq(&entity.world.load_full(), world)
    }

    pub(crate) fn has_new_riders(&self) -> bool {
        !self
            .vehicle
            .get_entity()
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    pub(crate) fn teleport(
        mut self,
        position: Vector3<f64>,
        yaw_delta: Option<f32>,
        pitch_delta: Option<f32>,
        destination: &Arc<World>,
    ) {
        if !self.riders.is_empty() {
            Entity::publish_portal_passengers(&self.vehicle);
        }
        for trip in std::mem::take(&mut self.riders) {
            if !self.is_current_in(destination) || !trip.is_valid() {
                continue;
            }
            let passenger = trip.entity.clone();
            let base = passenger.get_entity();
            let yaw = yaw_delta.map(|delta| base.yaw.load() + delta);
            let pitch = pitch_delta.map(|delta| base.pitch.load() + delta);
            base.portal_cooldown
                .store(base.default_portal_cooldown(), Relaxed);
            if passenger.get_player().is_some() {
                Entity::teleport_portal_player(
                    trip,
                    self.vehicle.clone(),
                    position,
                    yaw,
                    pitch,
                    destination,
                );
            } else if Entity::teleport_portal_entity(&passenger, position, yaw, pitch, destination)
                && trip.is_current_in(destination)
                && self.is_current_in(destination)
            {
                Entity::restore_portal_passenger(&self.vehicle, passenger);
            }
        }
    }
}

impl Drop for PortalPassengers {
    fn drop(&mut self) {
        // Rejected insertion restores only the untouched source relationship. A published
        // foreign vehicle, superseding action or changed rider owns its subsequent state.
        if !self.is_current_in(&self.source)
            || self
                .source
                .get_entity_by_id(self.vehicle.get_entity().entity_id)
                .is_none_or(|current| !Arc::ptr_eq(&current, &self.vehicle))
        {
            return;
        }
        for trip in &self.riders {
            if self.is_current_in(&self.source)
                && trip.is_valid()
                && Arc::ptr_eq(&trip.source, &self.source)
            {
                Entity::restore_portal_passenger(&self.vehicle, trip.entity.clone());
            }
        }
    }
}

impl Entity {
    pub(in crate::entity) fn teleport_portal_passengers(
        entity: &Self,
        position: Vector3<f64>,
        yaw_delta: Option<f32>,
        pitch_delta: Option<f32>,
        destination: &Arc<World>,
    ) {
        let passengers = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers {
            let base = passenger.get_entity();
            base.invalidate_portal_travel();
            let yaw = yaw_delta.map(|delta| base.yaw.load() + delta);
            let pitch = pitch_delta.map(|delta| base.pitch.load() + delta);
            base.portal_cooldown
                .store(base.default_portal_cooldown(), Relaxed);
            if passenger.get_player().is_some() {
                let previous_rotation = (base.yaw.load(), base.pitch.load());
                passenger.teleport(position, yaw, pitch, destination.clone());
                if Arc::ptr_eq(&base.world.load_full(), destination) {
                    Self::teleport_portal_passengers(
                        base,
                        base.pos.load(),
                        yaw.map(|_| base.yaw.load() - previous_rotation.0),
                        pitch.map(|_| base.pitch.load() - previous_rotation.1),
                        destination,
                    );
                }
            } else {
                Self::teleport_portal_entity(&passenger, position, yaw, pitch, destination);
            }
        }
    }

    fn teleport_portal_player(
        trip: PortalTravel,
        vehicle: Arc<dyn EntityBase>,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        destination: &Arc<World>,
    ) {
        let passenger = trip.entity.clone();
        let base = passenger.get_entity();
        let Some(player) = trip.source.get_player_by_id(base.entity_id) else {
            return;
        };
        if !std::ptr::eq(player.get_entity(), base) || !trip.is_valid() {
            return;
        }
        let Some(admission) = destination.admit_transfer() else {
            return;
        };
        let cancellation = trip.cancellation.clone();
        let client = player.client.clone();
        let destination = destination.clone();
        // The queued rider owns admission and cancellation through the callback and arrival.
        player.client.clone().spawn_task(async move {
            let _admission = admission;
            tokio::select! {
                biased;
                () = cancellation.cancelled() => {},
                () = client.await_close_interrupt() => {},
                () = async {
                    if let Some(applied) = player.teleport_world_for_portal(
                        destination, position, yaw, pitch, Some(&trip),
                    ).await
                        && trip.is_current_in(&applied.new_world)
                    {
                        Self::restore_portal_passenger(&vehicle, passenger);
                    }
                } => {},
            }
        });
    }

    fn restore_portal_passenger(vehicle: &Arc<dyn EntityBase>, passenger: Arc<dyn EntityBase>) {
        let base = passenger.get_entity();
        let world = base.world.load_full();
        if Arc::ptr_eq(&vehicle.get_entity().world.load_full(), &world)
            && !vehicle.get_entity().is_removed()
            && world
                .get_entity_by_id(vehicle.get_entity().entity_id)
                .is_some_and(|current| Arc::ptr_eq(&current, vehicle))
            && !base.is_removed()
            && base.get_vehicle().is_none()
        {
            *base
                .vehicle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(vehicle.clone());
            vehicle
                .get_entity()
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(passenger);
            Self::publish_portal_passengers(vehicle);
        }
    }

    fn publish_portal_passengers(vehicle: &Arc<dyn EntityBase>) {
        let entity = vehicle.get_entity();
        let ids: Vec<_> = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|rider| rider.get_entity().entity_id.into())
            .collect();
        let world = entity.world.load_full();
        if let Some(tracked) = world.entity_tracker.get_tracked_entity(entity.entity_id)
            && Arc::ptr_eq(&tracked.entity, vehicle)
        {
            // ServerEntity.sendChanges sends changed riding lists, including the empty list.
            tracked.send_to_tracking_players(
                &pumpkin_protocol::java::client::play::CSetPassengers::new(
                    entity.entity_id.into(),
                    &ids,
                ),
                &world,
            );
        }
    }
}
