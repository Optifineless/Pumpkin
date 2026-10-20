use super::{Entity, RemovalReason};
use std::sync::atomic::Ordering;

impl Entity {
    /// PlayerList.remove marks this instance before logout persistence; cleanup dispatches links.
    pub(crate) fn mark_removed_only(&self, reason: RemovalReason) -> bool {
        if self
            .removal_reason
            .compare_exchange(None, Some(reason))
            .is_err()
        {
            return false;
        }
        self.removed.store(true, Ordering::Release);
        true
    }

    /// Releases the links after the world lifecycle owner has committed removal.
    pub(crate) fn detach_riding_links_after_removal(&self, reason: RemovalReason) {
        if reason.should_destroy() {
            self.stop_riding_for_lifecycle();
        }
        self.eject_passengers_for_lifecycle();
    }

    /// PlayerList.respawn removes the old entity; Entity.setRemoved clears its Entity.unRide links.
    pub(crate) fn unride_for_respawn(&self) {
        // PlayerList.respawn retires the old ServerPlayer before publishing its replacement.
        self.stop_riding_for_lifecycle();
        self.eject_passengers_for_lifecycle();
    }

    fn stop_riding_for_lifecycle(&self) {
        if let Some(vehicle) = self.get_vehicle() {
            vehicle
                .get_entity()
                .remove_passenger_on_disconnect(self.entity_id);
        }
    }

    fn eject_passengers_for_lifecycle(&self) {
        // Entity.ejectPassengers snapshots first because each unlink mutates this list.
        let passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers {
            self.remove_passenger_on_disconnect(passenger.get_entity().entity_id);
        }
    }
}
