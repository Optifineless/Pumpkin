use super::{EntityBase, LivingEntity};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

const EPSILON: f32 = 1.0e-5;

// Mth.equal(double, double), used by Entity.move's horizontal collision checks.
fn equal(a: f64, b: f64) -> bool {
    (b - a).abs() < f64::from(EPSILON)
}

impl LivingEntity {
    // Player.getMoveSimulationType + LivingEntity.travelInAir: simulate motion on the server.
    // Pumpkin retains accepted client positions, so collision prediction changes only velocity.
    pub(super) fn travel_player_motion(&self, caller: &dyn EntityBase) {
        if self.entity.has_vehicle() {
            // Entity.rideTick clears motion; LivingEntity.rideTick resets fall distance.
            // Pumpkin's client-authoritative rider omits vanilla's one-tick travel remainder.
            self.entity.velocity.store(Vector3::default());
            self.fall_distance.store(0.0);
            return;
        }
        self.entity.check_zero_velo();
        let flying = caller.get_player().is_some_and(|player| {
            player
                .abilities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .flying
        });
        let original_y = self.entity.velocity.load().y;
        if !flying
            && (self.entity.touching_water.load(Relaxed) || self.entity.touching_lava.load(Relaxed))
        {
            self.travel_in_fluid(caller, self.entity.touching_water.load(Relaxed));
        } else {
            self.travel_in_air(caller);
        }
        if flying {
            // Player.travel restores flight Y after superclass travel.
            let mut motion = self.entity.velocity.load();
            motion.y = original_y * 0.6;
            self.entity.velocity.store(motion);
        }
    }

    #[expect(
        clippy::float_cmp,
        reason = "Entity.move compares vertical clipping exactly"
    )]
    pub(super) fn clip_player_motion(&self, caller: &dyn EntityBase) {
        let entity = &self.entity;
        let mut motion = entity.velocity.load();
        if !entity.no_physics.load(Relaxed) {
            let on_ground = entity.on_ground.load(Relaxed);
            let supporting = entity.supporting_block_pos.load();
            let horizontal_collision = entity.horizontal_collision.load(Relaxed);
            // An accepted landing already consumed the downward motion, as Entity.move does.
            if on_ground && motion.y < 0.0 {
                motion.y = 0.0;
            }
            let clipped = entity.adjust_movement_for_collisions(motion, caller);
            entity.on_ground.store(on_ground, Relaxed);
            entity.supporting_block_pos.store(supporting);
            entity
                .horizontal_collision
                .store(horizontal_collision, Relaxed);
            // Entity.restituteMovementAfterCollisions discards blocked velocity, not just distance.
            if !equal(motion.x, clipped.x) {
                motion.x = 0.0;
            }
            if motion.y != clipped.y {
                motion.y = 0.0;
            }
            if !equal(motion.z, clipped.z) {
                motion.z = 0.0;
            }
        }
        entity.velocity.store(motion);
    }
}
