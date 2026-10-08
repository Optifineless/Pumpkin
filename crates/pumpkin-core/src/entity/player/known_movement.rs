use super::{AtomicBool, AtomicCell, EntityBase, Ordering, Player, Vector3};

#[derive(Default)]
pub struct KnownMovement {
    movement: AtomicCell<Vector3<f64>>,
    received_this_tick: AtomicBool,
}

impl KnownMovement {
    // ServerGamePacketListenerImpl.handlePlayerKnownMovement / handleClientTickEnd.
    pub(crate) fn record(&self, movement: Vector3<f64>) {
        self.movement.store(movement);
        self.received_this_tick.store(true, Ordering::Relaxed);
    }

    /// Returns whether an accepted movement arrived during this client tick.
    pub(crate) fn finish_tick(&self) -> bool {
        let received = self.received_this_tick.swap(false, Ordering::Relaxed);
        if !received {
            self.movement.store(Vector3::new(0.0, 0.0, 0.0));
        }
        received
    }
}

impl Player {
    pub(crate) fn controls_vehicle(&self, vehicle: &dyn EntityBase) -> bool {
        // AbstractBoat.getControllingPassenger; other implemented vehicles have no controller.
        vehicle
            .cast_any()
            .is::<crate::entity::vehicle::boat::BoatEntity>()
            && vehicle
                .get_entity()
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .first()
                .is_some_and(|passenger| passenger.get_entity().entity_id == self.entity_id())
    }

    /// Returns accepted client movement for melee sweeps and projectile launch velocity.
    pub fn get_known_movement(&self) -> Vector3<f64> {
        // ServerPlayer.getKnownMovement uses the vehicle when this player is a passenger.
        if let Some(vehicle) = self.get_entity().get_vehicle()
            && !self.controls_vehicle(vehicle.as_ref())
        {
            return vehicle.get_entity().velocity.load();
        }
        self.known_movement.movement.load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::combat::is_sweep_attack;

    #[test]
    fn melee_sweep_classifies_the_last_accepted_movement_and_idle_tick() {
        let known = KnownMovement::default();
        let classify = || {
            let movement = known.movement.load();
            is_sweep_attack(
                true,
                true,
                true,
                movement.x * movement.x + movement.z * movement.z,
                0.1,
            )
        };
        known.record(Vector3::new(0.4, 0.0, 0.0));
        assert!(!classify());
        known.record(Vector3::new(0.0, 2.0, 0.0));
        assert!(classify());
        // Client tick end retains an accepted move until the next idle client tick.
        known.record(Vector3::new(0.3, 0.0, 0.0));
        assert!(known.finish_tick());
        assert!(!classify());
        assert!(!known.finish_tick());
        assert!(classify());
        let threshold = f64::from(0.1f32) * 2.5;
        assert!(!is_sweep_attack(
            true,
            true,
            true,
            threshold * threshold,
            0.1
        ));
        assert!(!is_sweep_attack(true, false, true, 0.0, 0.1));
    }
    #[test]
    fn known_movement_resets_only_after_a_client_tick_without_an_accepted_move() {
        let known = KnownMovement::default();
        known.record(Vector3::new(0.3, 0.4, -0.2));
        known.record(Vector3::new(0.1, 0.2, -0.3));
        assert!(known.finish_tick());
        assert_eq!(known.movement.load(), Vector3::new(0.1, 0.2, -0.3));
        assert!(!known.finish_tick());
        assert_eq!(known.movement.load(), Vector3::new(0.0, 0.0, 0.0));
        known.record(Vector3::new(0.5, 0.0, 0.0));
        known.record(Vector3::new(0.0, 0.0, 0.0));
        assert!(known.finish_tick());
        assert_eq!(known.movement.load(), Vector3::new(0.0, 0.0, 0.0));
    }
}
