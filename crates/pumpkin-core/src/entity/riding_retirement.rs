//! Entity.removePassenger: relationship leaf mutation before callbacks and packets.
use std::sync::{Arc, PoisonError};

use pumpkin_protocol::codec::var_int::VarInt;

use super::{Entity, EntityBase};

impl Entity {
    pub(super) fn unlink_passenger(
        &self,
        passenger_id: i32,
    ) -> (Option<Arc<dyn EntityBase>>, Vec<VarInt>) {
        let snapshot = self
            .passengers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let target = snapshot
            .iter()
            .find(|passenger| passenger.get_entity().entity_id == passenger_id);
        let (removed, remaining) = {
            let mut passengers = self
                .passengers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let removed = target.and_then(|target| {
                passengers
                    .iter()
                    .position(|current| Arc::ptr_eq(current, target))
                    .map(|index| passengers.remove(index))
            });
            (removed, passengers.clone())
        };
        if let Some(passenger) = &removed {
            passenger
                .get_entity()
                .vehicle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
        }
        let ids = remaining
            .iter()
            .map(|passenger| VarInt(passenger.get_entity().entity_id))
            .collect();
        (removed, ids)
    }
}
