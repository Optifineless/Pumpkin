use super::{EnderDragonEntity, EnderDragonPart};
use crate::entity::Entity;
use pumpkin_util::math::{
    boundingbox::{BoundingBox, EntityDimensions},
    cos, sin,
    vector3::Vector3,
    wrap_degrees,
};

// EnderDragon constructor / EnderDragonPart.getDimensions (Java 26.3).
pub(super) const PART_SIZES: [(f32, f32); 8] = [
    (1.0, 1.0),
    (3.0, 3.0),
    (5.0, 3.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (4.0, 2.0),
    (4.0, 2.0),
];

pub(super) fn set_dimensions(entity: &Entity, width: f32, height: f32) {
    let dimensions = EntityDimensions::new(width, height, height * 0.85);
    entity.entity_dimension.store(dimensions);
    let pos = entity.pos.load();
    entity
        .bounding_box
        .store(BoundingBox::new_from_pos(pos.x, pos.y, pos.z, &dimensions));
}

impl EnderDragonEntity {
    pub(super) fn tick_parts(&self) {
        let history = *self
            .flight_history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p5 = history.get(5);
        let p10 = history.get(10);
        let p0 = history.get(0);

        // EnderDragon.aiStep:261-269 keeps these offset calculations in floats.
        let tilt = (p5.y - p10.y) as f32 * 10.0 * (std::f32::consts::PI / 180.0);
        let cc_tilt = cos(tilt);
        let ss_tilt = sin(tilt);
        let yaw = self.mob_entity.living_entity.entity.yaw.load();
        let rot1 = yaw * (std::f32::consts::PI / 180.0);
        let ss1 = sin(rot1);
        let cc1 = cos(rot1);

        self.tick_part(
            &self.parts[2],
            f64::from(ss1 * 0.5),
            0.0,
            f64::from(-cc1 * 0.5),
        );
        self.tick_part(
            &self.parts[6],
            f64::from(cc1 * 4.5),
            2.0,
            f64::from(ss1 * 4.5),
        );
        self.tick_part(
            &self.parts[7],
            f64::from(cc1 * -4.5),
            2.0,
            f64::from(ss1 * -4.5),
        );

        // EnderDragon.aiStep:270-291 uses new wings and the previous head/neck.
        // The history lock is released before any contact callbacks.
        self.handle_part_collisions();

        let head_y_offset = if self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_sitting()
        {
            -1.0
        } else {
            (p5.y - p0.y) as f32
        };
        let yaw_accel = *self
            .yaw_rot_accel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let rot2 = yaw * (std::f32::consts::PI / 180.0) - yaw_accel * 0.01;
        let ss2 = sin(rot2);
        let cc2 = cos(rot2);
        self.tick_part(
            &self.parts[0],
            f64::from(ss2 * 6.5 * cc_tilt),
            f64::from(head_y_offset + ss_tilt * 6.5),
            f64::from(-cc2 * 6.5 * cc_tilt),
        );
        self.tick_part(
            &self.parts[1],
            f64::from(ss2 * 5.5 * cc_tilt),
            f64::from(head_y_offset + ss_tilt * 5.5),
            f64::from(-cc2 * 5.5 * cc_tilt),
        );

        for i in 0..3 {
            let pi = history.get(12 + i * 2);
            let rot = rot1 + wrap_degrees(pi.y_rot - p5.y_rot) * (std::f32::consts::PI / 180.0);
            let ss = sin(rot);
            let cc = cos(rot);
            let dd = (i + 1) as f32 * 2.0;
            self.tick_part(
                &self.parts[3 + i as usize],
                f64::from(-(ss1 * 1.5 + ss * dd) * cc_tilt),
                pi.y - p5.y - f64::from((dd + 1.5) * ss_tilt) + 1.5,
                f64::from((cc1 * 1.5 + cc * dd) * cc_tilt),
            );
        }
    }

    fn tick_part(&self, part: &EnderDragonPart, x: f64, y: f64, z: f64) {
        // EnderDragon.tickPart:338-340.
        let pos = self.mob_entity.living_entity.entity.pos.load();
        part.entity
            .set_pos(Vector3::new(pos.x + x, pos.y + y, pos.z + z));
    }
}
