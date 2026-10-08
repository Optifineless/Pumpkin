//! `PhantomCircleAroundAnchorGoal`; attack goals can replace `move_target_point` for swoops.
use crate::entity::{
    EntityBase,
    ai::goal::{Controls, Goal},
    mob::{Mob, phantom::PhantomEntity},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

#[derive(Default)]
pub struct PhantomCircleAroundAnchorGoal {
    angle: f32,
    distance: f32,
    height: f32,
    clockwise: f32,
}
impl PhantomCircleAroundAnchorGoal {
    fn select_next(&mut self, phantom: &PhantomEntity) {
        // Phantom.finalizeSpawn supplies blockPosition().above(5) for idle flight.
        let anchor = phantom
            .anchor_point
            .load()
            .unwrap_or_else(|| phantom.get_entity().block_pos.load().add(0, 5, 0));
        phantom.anchor_point.store(Some(anchor));
        self.angle += self.clockwise * 15.0 * std::f32::consts::PI / 180.0;
        phantom.move_target_point.store(Vector3::new(
            f64::from(anchor.0.x) + f64::from(self.distance * self.angle.cos()),
            f64::from(anchor.0.y) + f64::from(-4.0 + self.height),
            f64::from(anchor.0.z) + f64::from(self.distance * self.angle.sin()),
        ));
    }
}
impl Goal for PhantomCircleAroundAnchorGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<PhantomEntity>()
            .is_some_and(|p| {
                !p.swooping.load(std::sync::atomic::Ordering::Relaxed)
                    || p.mob_entity.get_target().is_none()
            })
    }
    fn start(&mut self, mob: &dyn Mob) {
        let Some(p) = mob.cast_any().downcast_ref::<PhantomEntity>() else {
            return;
        };
        self.distance = 5.0 + rand::random::<f32>() * 10.0;
        self.height = -4.0 + rand::random::<f32>() * 9.0;
        self.clockwise = if rand::random() { 1.0 } else { -1.0 };
        self.select_next(p);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(p) = mob.cast_any().downcast_ref::<PhantomEntity>() else {
            return;
        };
        if rand::random_range(0..self.get_tick_count(350)) == 0 {
            self.height = -4.0 + rand::random::<f32>() * 9.0;
        }
        if rand::random_range(0..self.get_tick_count(250)) == 0 {
            self.distance += 1.0;
            if self.distance > 15.0 {
                self.distance = 5.0;
                self.clockwise = -self.clockwise;
            }
        }
        if rand::random_range(0..self.get_tick_count(450)) == 0 {
            self.angle = rand::random::<f32>() * 2.0 * std::f32::consts::PI;
            self.select_next(p);
        }
        let entity = p.get_entity();
        if (p.move_target_point.load() - entity.pos.load()).length_squared() < 4.0 {
            self.select_next(p);
        }
        let world = entity.world.load();
        let pos = entity.block_pos.load();
        if p.move_target_point.load().y < entity.pos.load().y
            && !world.get_block_state(&pos.down()).is_air()
        {
            self.height = self.height.max(1.0);
            self.select_next(p);
        }
        if p.move_target_point.load().y > entity.pos.load().y
            && !world
                .get_block_state(&BlockPos::new(pos.0.x, pos.0.y + 1, pos.0.z))
                .is_air()
        {
            self.height = self.height.min(-1.0);
            self.select_next(p);
        }
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
