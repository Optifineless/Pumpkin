use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::mob::{Mob, MobEntity};
use pumpkin_data::Block;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    #[default]
    Wait,
    MoveTo,
    Strafe,
    Jumping,
}

pub struct MoveControl {
    pub wanted_x: f64,
    pub wanted_y: f64,
    pub wanted_z: f64,
    pub speed_modifier: f64,
    pub strafe_forwards: f32,
    pub strafe_right: f32,
    pub operation: Operation,
}

impl Default for MoveControl {
    fn default() -> Self {
        Self {
            wanted_x: 0.0,
            wanted_y: 0.0,
            wanted_z: 0.0,
            speed_modifier: 0.0,
            strafe_forwards: 0.0,
            strafe_right: 0.0,
            operation: Operation::Wait,
        }
    }
}

impl Control for MoveControl {}

impl MoveControlTrait for MoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let living_entity = &mob_entity.living_entity;
        let entity = &living_entity.entity;
        if self.operation == Operation::Strafe {
            self.tick_strafe(mob_entity);
        } else if self.operation == Operation::MoveTo {
            self.operation = Operation::Wait;
            let pos = entity.pos.load();
            let xd = self.wanted_x - pos.x;
            let zd = self.wanted_z - pos.z;
            let yd = self.wanted_y - pos.y;
            let dd = xd * xd + yd * yd + zd * zd;

            if dd < 2.5000003E-7 {
                living_entity
                    .movement_input
                    .store(Vector3::new(0.0, 0.0, 0.0));
                return;
            }

            let y_rot_d = (zd.atan2(xd).to_degrees() as f32) - 90.0;
            entity
                .yaw
                .store(self.rotlerp(entity.yaw.load(), y_rot_d, 90.0));

            let movement_speed = living_entity.get_attribute_value(&Attributes::MOVEMENT_SPEED);
            let speed = self.speed_modifier * movement_speed;
            mob_entity.movement_speed.store(speed as f32);
            living_entity
                .movement_input
                .store(Vector3::new(0.0, 0.0, speed));

            if Self::jump_if_needed(
                mob_entity,
                Vector3::new(self.wanted_x, self.wanted_y, self.wanted_z),
            ) {
                self.operation = Operation::Jumping;
            }
        } else if self.operation == Operation::Jumping {
            let movement_speed = living_entity.get_attribute_value(&Attributes::MOVEMENT_SPEED);
            let speed = self.speed_modifier * movement_speed;
            mob_entity.movement_speed.store(speed as f32);
            living_entity
                .movement_input
                .store(Vector3::new(0.0, 0.0, speed));

            if entity.on_ground.load(Ordering::Relaxed)
                || entity.touching_water.load(Ordering::Relaxed)
                || entity.touching_lava.load(Ordering::Relaxed)
            {
                self.operation = Operation::Wait;
            }
        } else {
            let input = living_entity.movement_input.load();
            living_entity
                .movement_input
                .store(Vector3::new(input.x, input.y, 0.0));
        }
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.wanted_x = x;
        self.wanted_y = y;
        self.wanted_z = z;
        self.speed_modifier = speed_modifier;
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }

    fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = Operation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }

    fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }
}

impl MoveControl {
    // MoveControl.tick STRAFE probes a speed-scaled, rotated step before applying raw input.
    fn tick_strafe(&mut self, mob: &MobEntity) {
        let living = &mob.living_entity;
        let speed = self.speed_modifier as f32
            * living.get_attribute_value(&Attributes::MOVEMENT_SPEED) as f32;
        let (dx, dz) = strafe_probe(
            self.strafe_forwards,
            self.strafe_right,
            speed,
            living.entity.yaw.load(),
        );
        let pos = living.entity.pos.load();
        let probe = pumpkin_util::math::position::BlockPos::floored(
            pos.x + f64::from(dx),
            pos.y,
            pos.z + f64::from(dz),
        );
        let walkable = mob
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .path_type_at(living, probe)
            == crate::entity::ai::pathfinder::node::PathType::Walkable;
        if !walkable {
            self.strafe_forwards = 1.0;
            self.strafe_right = 0.0;
        }
        mob.set_speed(speed);
        let input = living.movement_input.load();
        living.movement_input.store(Vector3::new(
            f64::from(self.strafe_right),
            input.y,
            f64::from(self.strafe_forwards),
        ));
        self.operation = Operation::Wait;
    }

    // Vanilla MoveControl.tick's MOVE_TO jump request after navigation supplies a waypoint.
    pub fn jump_if_needed(mob: &MobEntity, wanted: Vector3<f64>) -> bool {
        let living = &mob.living_entity;
        let entity = &living.entity;
        let pos = entity.pos.load();
        let delta = wanted - pos;
        let block_pos = entity.block_pos.load();
        let world = entity.world.load();
        let state = world.get_block_state(&block_pos);
        let block = Block::from_state_id(state.id);
        let above_step = delta.y > living.get_attribute_value(&Attributes::STEP_HEIGHT)
            && delta.x * delta.x + delta.z * delta.z < 1.0f64.max(f64::from(entity.width()));
        let inside_shape = state
            .get_block_collision_shapes_at(&block_pos)
            .any(|shape| pos.y < shape.max.y + f64::from(block_pos.0.y))
            && !block.has_tag(&tag::Block::MINECRAFT_DOORS)
            && !block.has_tag(&tag::Block::MINECRAFT_FENCES);

        if above_step || inside_shape {
            mob.jump_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .jump();
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }

    #[must_use]
    pub const fn get_speed_modifier(&self) -> f64 {
        self.speed_modifier
    }

    pub fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.wanted_x = x;
        self.wanted_y = y;
        self.wanted_z = z;
        self.speed_modifier = speed_modifier;
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }

    pub const fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = Operation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }
}

// MoveControl.tick uses forward/right axes for this probe (not moveRelative's input axes).
#[expect(
    clippy::imprecise_flops,
    reason = "MoveControl.tick rounds each float operation before sqrt"
)]
fn strafe_probe(forward: f32, right: f32, speed: f32, yaw: f32) -> (f32, f32) {
    let scale = speed / (forward * forward + right * right).sqrt().max(1.0);
    let (sin, cos) = yaw.to_radians().sin_cos();
    (
        forward * scale * cos - right * scale * sin,
        right * scale * cos + forward * scale * sin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strafe_probe_is_scaled_and_rotated() {
        let (x, z) = strafe_probe(1.0, 1.0, 0.25, 90.0);
        assert!((x + 0.176_776_69).abs() < 1e-7);
        assert!((z - 0.176_776_69).abs() < 1e-7);
    }

    #[test]
    fn movement_rotation_normalizes_but_look_rotation_does_not() {
        let control = MoveControl::default();
        assert_eq!(control.rotlerp(350.0, 20.0, 90.0), 20.0);
        assert_eq!(control.rotlerp(10.0, -20.0, 90.0), 340.0);
        assert_eq!(control.change_angle(350.0, 20.0, 90.0), 380.0);
    }
}
