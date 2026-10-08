use crate::entity::ai::control::Control;
use crate::entity::mob::Mob;
use pumpkin_util::math::rotate_if_necessary;

// BodyRotationControl's timing constants.
const HEAD_STABLE_ANGLE: f32 = 15.0;
const DELAY_UNTIL_STARTING_TO_FACE_FORWARD: i32 = 10;
const HOW_LONG_IT_TAKES_TO_FACE_FORWARD: f32 = 10.0;

pub struct BodyRotationControl {
    head_stable_time: i32,
    last_stable_y_head_rot: f32,
}

impl Default for BodyRotationControl {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyRotationControl {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            head_stable_time: 0,
            last_stable_y_head_rot: 0.0,
        }
    }

    // LivingEntity.tick -> Mob.tickHeadTurn, after travel.
    pub fn client_tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let moving = Self::is_moving(mob);
        let passenger = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .is_some_and(|p| p.get_mob().is_some());
        let (body, head) = self.update(
            (
                entity.yaw.load(),
                entity.body_yaw.load(),
                entity.head_yaw.load(),
            ),
            mob.get_max_head_rotation(),
            moving,
            passenger,
        );
        entity.body_yaw.store(body);
        entity.head_yaw.store(head);
    }

    fn update(
        &mut self,
        angles: (f32, f32, f32),
        max_rotation: f32,
        moving: bool,
        mob_passenger: bool,
    ) -> (f32, f32) {
        let (yaw, mut body, mut head) = angles;
        if moving {
            body = yaw;
            head = rotate_if_necessary(head, body, max_rotation);
            self.last_stable_y_head_rot = head;
            self.head_stable_time = 0;
        } else if !mob_passenger {
            if (head - self.last_stable_y_head_rot).abs() > HEAD_STABLE_ANGLE {
                self.head_stable_time = 0;
                self.last_stable_y_head_rot = head;
                body = rotate_if_necessary(body, head, max_rotation);
            } else {
                self.head_stable_time += 1;
                if self.head_stable_time > DELAY_UNTIL_STARTING_TO_FACE_FORWARD {
                    let fraction = ((self.head_stable_time - DELAY_UNTIL_STARTING_TO_FACE_FORWARD)
                        as f32
                        / HOW_LONG_IT_TAKES_TO_FACE_FORWARD)
                        .clamp(0.0, 1.0);
                    body = rotate_if_necessary(body, head, max_rotation * (1.0 - fraction));
                }
            }
        }
        (body, head)
    }

    fn is_moving(mob: &dyn Mob) -> bool {
        let entity = &mob.get_mob_entity().living_entity.entity;
        // BodyRotationControl.isMoving uses displacement, including collision clipping.
        let delta = entity.pos.load() - entity.last_pos.load();
        delta.x * delta.x + delta.z * delta.z > f64::from(2.500_000_3e-7f32)
    }
}

impl Control for BodyRotationControl {}

#[cfg(test)]
mod tests {
    use super::BodyRotationControl;
    #[test]
    fn stationary_body_settles_unless_carrying_a_mob() {
        let mut control = BodyRotationControl::new();
        let (mut body, head) = control.update((0.0, 0.0, 60.0), 75.0, false, false);
        for _ in 0..20 {
            (body, _) = control.update((0.0, body, head), 75.0, false, false);
        }
        assert!((body - 60.0).abs() < f32::EPSILON);
        for _ in 0..30 {
            (body, _) = control.update((0.0, body, 0.0), 75.0, false, true);
        }
        assert!((body - 60.0).abs() < f32::EPSILON);
        let (body, head) = control.update((180.0, body, 0.0), 75.0, true, true);
        assert!((body - 180.0).abs() < f32::EPSILON);
        assert!((head - 255.0).abs() < f32::EPSILON);
    }
}
