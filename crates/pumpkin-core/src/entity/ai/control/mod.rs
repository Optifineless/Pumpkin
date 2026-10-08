use crate::entity::mob::Mob;
use pumpkin_util::math::subtract_angles;

pub mod body_rotation_control;
pub mod fish_move_control;
pub mod flying_move_control;
pub mod jump_control;
pub mod look_control;
pub mod move_control;
pub mod smooth_swimming_look_control;
pub mod smooth_swimming_move_control;

pub trait Control: Send + Sync {
    fn change_angle(&self, start: f32, end: f32, max_change: f32) -> f32 {
        let i = subtract_angles(start, end);
        let j = i.clamp(-max_change, max_change);
        start + j
    }
}

pub trait MoveControlTrait: Control {
    // MoveControl.rotlerp normalizes the result; Control.rotateTowards deliberately does not.
    fn rotlerp(&self, start: f32, end: f32, max_change: f32) -> f32 {
        let result = start + subtract_angles(start, end).clamp(-max_change, max_change);
        if result < 0.0 {
            result + 360.0
        } else if result > 360.0 {
            result - 360.0
        } else {
            result
        }
    }

    fn tick(&mut self, mob: &dyn Mob);

    fn set_wanted_position(&mut self, _x: f64, _y: f64, _z: f64, _speed_modifier: f64) {}

    fn strafe(&mut self, _forward: f32, _right: f32) {}

    /// Last requested destination and speed, including while waiting for a hop.
    fn wanted_position(&self) -> (pumpkin_util::math::vector3::Vector3<f64>, f64) {
        (
            pumpkin_util::math::vector3::Vector3::new(0.0, 0.0, 0.0),
            0.0,
        )
    }

    fn has_wanted(&self) -> bool {
        false
    }

    /// Guardian's published moving state, independent of the movement-speed attribute.
    fn is_moving(&self) -> bool {
        false
    }
}

pub mod turtle_move_control;

pub mod drowned_move_control;
pub mod guardian_move_control;

pub mod rabbit_move_control;

pub mod ghast_move_control;

pub mod phantom_move_control;
pub mod vex_move_control;
