//! Entity.startRiding/removeVehicle/addPassenger/removePassenger link publication.
use std::sync::{Arc, atomic::AtomicU64, atomic::Ordering};

use super::{Entity, EntityBase};

const VEHICLE: u64 = 1;
const PASSENGERS: u64 = 2;
const FLAGS: u64 = VEHICLE | PASSENGERS;
const NEXT_GENERATION: u64 = FLAGS + 1;

#[derive(Default)]
pub(super) struct RidingAdmission(AtomicU64);

impl RidingAdmission {
    pub(super) fn load(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub(super) const fn is_unmounted(state: u64) -> bool {
        state & FLAGS == 0
    }

    fn change(&self, flag: u64, present: bool) {
        let _ = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                let generation = (state & !FLAGS).wrapping_add(NEXT_GENERATION);
                let flags = if present { state | flag } else { state & !flag };
                Some(generation | (flags & FLAGS))
            });
    }
}

impl Entity {
    /// Publishes the riding flag before adding a vehicle; clears it after unlinking.
    pub(crate) fn set_vehicle_link(&self, vehicle: Option<Arc<dyn EntityBase>>) {
        let mut link = self
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if vehicle.is_some() {
            self.riding_admission.change(VEHICLE, true);
            *link = vehicle;
        } else {
            *link = None;
            self.riding_admission.change(VEHICLE, false);
        }
    }

    /// Adds a passenger while holding its list lock; published callers also own mutation admission.
    pub(crate) fn push_passenger_link(&self, passenger: Arc<dyn EntityBase>) {
        let mut passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.insert_passenger_link(&mut passengers, passenger);
    }

    /// Adds a link under this entity's passenger-list lock, publishing the flag first.
    pub(crate) fn insert_passenger_link(
        &self,
        passengers: &mut Vec<Arc<dyn EntityBase>>,
        passenger: Arc<dyn EntityBase>,
    ) {
        self.riding_admission.change(PASSENGERS, true);
        passengers.push(passenger);
    }

    /// Finishes a removal under the passenger-list lock, after the list has been edited.
    pub(crate) fn removed_passenger_link(&self, passenger: &Arc<dyn EntityBase>, empty: bool) {
        passenger.get_entity().set_vehicle_link(None);
        self.riding_admission.change(PASSENGERS, !empty);
    }

    /// Breaks an unpublished rejected tree without packets or plugin events.
    pub(crate) fn take_unpublished_passenger_links(&self) -> Vec<Arc<dyn EntityBase>> {
        let mut passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let removed = std::mem::take(&mut *passengers);
        self.riding_admission.change(PASSENGERS, false);
        removed
    }
}
