use super::{
    Control, MoveControlTrait,
    move_control::{MoveControl, Operation},
};
use crate::entity::{EntityBase, mob::Mob, passive::rabbit::RabbitEntity};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{PoisonError, Weak, atomic::Ordering};

/// Rabbit.RabbitMoveControl retains the next hop's speed while waiting on the ground.
pub struct RabbitMoveControl {
    rabbit: Weak<RabbitEntity>,
    inner: MoveControl,
    next_jump_speed: f64,
}

impl RabbitMoveControl {
    #[must_use]
    pub fn new(rabbit: Weak<RabbitEntity>) -> Self {
        let mut inner = MoveControl::default();
        // Rabbit's constructor calls setSpeedModifier(0), publishing an initial MOVE_TO.
        inner.set_wanted_position(0.0, 0.0, 0.0, 0.0);
        Self {
            rabbit,
            inner,
            next_jump_speed: 0.0,
        }
    }

    // RabbitMoveControl.setWantedPosition applies the water override even to a zero-speed request.
    fn set_wanted_position_in_water(&mut self, position: Vector3<f64>, speed: f64, in_water: bool) {
        let speed = if in_water { 1.5 } else { speed };
        self.inner
            .set_wanted_position(position.x, position.y, position.z, speed);
        if speed > 0.0 {
            self.next_jump_speed = speed;
        }
    }
}

impl Control for RabbitMoveControl {}
impl MoveControlTrait for RabbitMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let m = mob.get_mob_entity();
        let e = &m.living_entity.entity;
        let requested = m
            .jump_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .has_request();
        let speed = if e.on_ground.load(Ordering::Relaxed)
            && !m.living_entity.jumping.load(Ordering::Relaxed)
            && !requested
        {
            Some(0.0)
        } else if self.has_wanted() || self.inner.operation == Operation::Jumping {
            Some(self.next_jump_speed)
        } else {
            None
        };
        if let Some(speed) = speed {
            m.navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_speed(speed);
            self.set_wanted_position(
                self.inner.wanted_x,
                self.inner.wanted_y,
                self.inner.wanted_z,
                speed,
            );
        }
        self.inner.tick(mob);
    }
    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        let in_water = self
            .rabbit
            .upgrade()
            .is_some_and(|rabbit| rabbit.get_entity().touching_water.load(Ordering::Relaxed));
        self.set_wanted_position_in_water(Vector3::new(x, y, z), speed, in_water);
    }
    fn has_wanted(&self) -> bool {
        self.inner.has_wanted()
    }
    fn wanted_position(&self) -> (Vector3<f64>, f64) {
        (
            Vector3::new(
                self.inner.wanted_x,
                self.inner.wanted_y,
                self.inner.wanted_z,
            ),
            self.inner.speed_modifier,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_requests_keep_hop_speed_on_land_but_use_swimming_speed_in_water() {
        let mut control = RabbitMoveControl::new(Weak::new());
        let destination = Vector3::new(3.0, 4.0, 5.0);
        control.set_wanted_position_in_water(destination, 2.2, false);
        control.set_wanted_position_in_water(destination, 0.0, false);
        assert_eq!(control.wanted_position(), (destination, 0.0));
        assert!((control.next_jump_speed - 2.2).abs() < f64::EPSILON);

        control.set_wanted_position_in_water(destination, 0.0, true);
        assert_eq!(control.wanted_position(), (destination, 1.5));
        assert!((control.next_jump_speed - 1.5).abs() < f64::EPSILON);
        control.set_wanted_position_in_water(destination, 0.6, false);
        assert_eq!(control.wanted_position(), (destination, 0.6));
    }
}
