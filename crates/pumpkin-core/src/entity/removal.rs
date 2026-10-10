use super::{Entity, RemovalReason};
use std::sync::atomic::Ordering;

impl Entity {
    // Entity.removePassenger hardcodes boardingCooldown = 60 (vanilla 26.3:2601).
    pub(super) const BOARDING_COOLDOWN: i32 = 60;

    /// Marks the instance removed once and releases its riders before world callbacks.
    /// Chunk unload callers must capture the riding tree before retiring its instances.
    pub(crate) fn set_removed(&self, reason: RemovalReason) -> bool {
        // Entity.setRemoved preserves the first reason, stops destructive removals
        // riding their parent, and ejects passengers for every removal reason.
        if self
            .removal_reason
            .compare_exchange(None, Some(reason))
            .is_err()
        {
            return false;
        }
        self.removed.store(true, Ordering::Release);
        if reason.should_destroy() {
            self.stop_riding_on_removal();
        }
        self.eject_passengers_on_removal();
        true
    }

    /// Releases both riding directions at the end of a player life, without teleporting.
    pub(crate) fn unride_on_lifecycle_end(&self) {
        // Entity.unRide / PlayerList.respawn: Pumpkin reuses the old Player instance.
        self.stop_riding_on_removal();
        self.eject_passengers_on_removal();
    }

    fn stop_riding_on_removal(&self) {
        if let Some(vehicle) = self.get_vehicle() {
            vehicle
                .get_entity()
                .remove_passenger_on_disconnect(self.entity_id);
        }
    }

    fn eject_passengers_on_removal(&self) {
        // Snapshot before dismounting: removing a rider takes the same passengers mutex.
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
