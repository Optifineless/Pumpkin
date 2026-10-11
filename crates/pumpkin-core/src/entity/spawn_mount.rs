//! `EntityType.loadEntityRecursive` riding links for an unpublished entity tree.

use super::EntityBase;
use std::sync::{Arc, PoisonError};

/// Breaks unpublished riding links on rejection; call `keep_links` after ownership is transferred.
pub struct UnpublishedRidingTree {
    root: Arc<dyn EntityBase>,
    keep: bool,
}

impl UnpublishedRidingTree {
    pub fn new(root: &Arc<dyn EntityBase>) -> Self {
        Self {
            root: root.clone(),
            keep: false,
        }
    }
    pub const fn keep_links(&mut self) {
        self.keep = true;
    }
}

impl Drop for UnpublishedRidingTree {
    fn drop(&mut self) {
        if !self.keep {
            let mut pending = vec![self.root.clone()];
            while let Some(entity) = pending.pop() {
                let base = entity.get_entity();
                pending.extend(base.take_unpublished_passenger_links());
                base.set_vehicle_link(None);
            }
        }
    }
}

/// Links a configured passenger without events or packets; insertion validates the entire tree.
pub fn attach_unpublished(vehicle: &Arc<dyn EntityBase>, passenger: Arc<dyn EntityBase>) {
    passenger
        .get_entity()
        .set_vehicle_link(Some(vehicle.clone()));
    vehicle.get_entity().push_passenger_link(passenger);
}

/// Checks mount events before publishing any member; cancellation rejects the whole spawn.
pub fn accept_mount(vehicle: &Arc<dyn EntityBase>, passenger: &Arc<dyn EntityBase>) -> bool {
    let mut mount = crate::plugin::api::events::entity::entity_mount::EntityMountEvent::new(
        passenger.get_entity().entity_id,
        vehicle.get_entity().entity_id,
    );
    let mut enter = crate::plugin::api::events::vehicle::vehicle_enter::VehicleEnterEvent::new(
        vehicle.get_entity().entity_id,
        passenger.get_entity().entity_id,
    );
    if let Some(server) = vehicle.get_entity().world.load().server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut mount);
        server.plugin_manager.fire_blocking(&server, &mut enter);
    }
    !mount.cancelled && !enter.cancelled
}

pub fn publish_passengers(vehicle: &Arc<dyn EntityBase>) {
    let ids: Vec<_> = vehicle
        .get_entity()
        .passengers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|passenger| pumpkin_protocol::codec::var_int::VarInt(passenger.get_entity().entity_id))
        .collect();
    if !ids.is_empty() {
        let entity = vehicle.get_entity();
        entity.world.load().broadcast_to_chunk(
            entity.chunk_pos.load(),
            &pumpkin_protocol::java::client::play::CSetPassengers::new(
                pumpkin_protocol::codec::var_int::VarInt(entity.entity_id),
                &ids,
            ),
        );
    }
}

/// Defers a freshly finalized mount until the rider is accepted or retained in its generation chunk.
pub fn queue_spawn_mount(rider: &Arc<dyn EntityBase>, mount: Arc<dyn EntityBase>) {
    if let Some(mob) = rider.get_mob() {
        *mob.get_mob_entity()
            .pending_spawn_mount
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(mount);
    }
}

/// Materializes a fresh finalizer's mount as the root of the unpublished riding tree.
pub fn spawn_root(entity: &Arc<dyn EntityBase>) -> Arc<dyn EntityBase> {
    let mount = entity.get_mob().and_then(|mob| {
        mob.get_mob_entity()
            .pending_spawn_mount
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    });
    mount.map_or_else(
        || entity.clone(),
        |mount| {
            attach_unpublished(&mount, entity.clone());
            mount
        },
    )
}

/// `Zombie.finalizeSpawn` uses `EntitySelector.ENTITY_NOT_BEING_RIDDEN`.
pub fn available_chicken(entity: &Arc<dyn EntityBase>) -> bool {
    let base = entity.get_entity();
    base.entity_type == &pumpkin_data::entity::EntityType::CHICKEN
        && base.is_alive()
        && entity
            .get_living_entity()
            .is_none_or(|living| living.health.load() > 0.0)
        && !base.has_passengers()
        && base.get_vehicle().is_none()
}

/// Records a live candidate without changing its jockey flag or riding links.
pub fn queue_existing_chicken(rider: &Arc<dyn EntityBase>, mount: Arc<dyn EntityBase>) {
    if let Some(mob) = rider.get_mob() {
        *mob.get_mob_entity()
            .pending_existing_mount
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(mount);
    }
}

pub fn take_existing_chicken(rider: &Arc<dyn EntityBase>) -> Option<Arc<dyn EntityBase>> {
    rider
        .get_mob()?
        .get_mob_entity()
        .pending_existing_mount
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

pub fn attach_accepted_chicken(mount: &Arc<dyn EntityBase>, rider: Arc<dyn EntityBase>) {
    if !available_chicken(mount) {
        return;
    }
    attach_unpublished(mount, rider);
    if let Some(mob) = mount.get_mob() {
        mob.set_chicken_jockey(true);
    }
}

/// Includes finalizer-created passengers in UUID admission and cancellation of the whole tree.
pub fn materialize_pending_riders(entity: &Arc<dyn EntityBase>) {
    if let Some(mob) = entity.get_mob() {
        for rider in mob.get_mob_entity().take_pending_riders() {
            attach_unpublished(entity, rider);
        }
    }
}
