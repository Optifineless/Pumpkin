use super::{FishingBobberEntity, RandomExt};
use crate::{plugin::api::events::player::fish::PlayerFishState, world::World};
use pumpkin_data::{
    Block,
    particle::Particle,
    sound::{Sound, SoundCategory},
    tracked_data,
};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    random::RandomImpl,
};
use std::sync::atomic::Ordering::Relaxed;

// FishingHook.catchingFish: weather changes the waiting/approach clock, never the bite window.
pub(super) fn fishing_speed(
    raining: bool,
    sky_visible: bool,
    rain_roll: f32,
    sky_roll: f32,
) -> i32 {
    1 + i32::from(rain_roll < 0.25 && raining) - i32::from(sky_roll < 0.5 && !sky_visible)
}

impl FishingBobberEntity {
    // FishingHook.catchingFish, with particle branches kept in their own helpers.
    pub(super) fn catching_fish(
        &self,
        world: &World,
        block_pos: &BlockPos,
        velocity: &mut Vector3<f64>,
    ) {
        let above = block_pos.up();
        let speed = fishing_speed(
            world.is_raining_at(&above),
            world.can_see_sky(&above),
            rand::random(),
            rand::random(),
        );
        let nibble = self.bite_countdown.load(Relaxed);
        if nibble > 0 {
            // Gate vanilla FishingHook.catchingFish's expiry before any state changes.
            if nibble == 1
                && self
                    .fire_fish_event(PlayerFishState::FailedAttempt, None, self.hand, 0)
                    .is_none()
            {
                return;
            }
            self.bite_countdown.store(nibble - 1, Relaxed);
            if nibble == 1 {
                self.wait_countdown.store(0, Relaxed);
                self.hook_countdown.store(0, Relaxed);
                self.entity
                    .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, false);
            }
        } else if self.hook_countdown.load(Relaxed) > 0 {
            let hooked = self.hook_countdown.load(Relaxed) - speed;
            self.hook_countdown.store(hooked, Relaxed);
            if hooked > 0 {
                self.approach_particles(world, hooked);
            } else {
                self.start_bite(world, velocity);
            }
        } else if self.wait_countdown.load(Relaxed) > 0 {
            let wait = self.wait_countdown.load(Relaxed) - speed;
            self.wait_countdown.store(wait, Relaxed);
            self.tease_particles(world, wait);
            if wait <= 0 {
                self.fish_angle.store(rand::random::<f32>() * 360.0);
                self.hook_countdown
                    .store(rand::random_range(20..=80), Relaxed);
            }
        } else {
            self.wait_countdown
                .store(rand::random_range(100..=600) - self.lure_speed, Relaxed);
        }
    }

    fn start_bite(&self, world: &World, velocity: &mut Vector3<f64>) {
        if self
            .fire_fish_event(PlayerFishState::Bite, None, self.hand, 0)
            .is_none()
        {
            return;
        }
        let pos = self.entity.pos.load();
        world.play_sound_fine(
            Sound::EntityFishingBobberSplash,
            SoundCategory::Neutral,
            &pos,
            0.25,
            rand::rng().triangle(1.0, 0.4) as f32,
        );
        let width = self.entity.width();
        let count = (1.0 + width * 20.0) as i32;
        let particle_pos = pos.add_raw(0.0, 0.5, 0.0);
        let offset = Vector3::new(width, 0.0, width);
        world.spawn_particle(particle_pos, offset, 0.2, count, Particle::Bubble);
        world.spawn_particle(particle_pos, offset, 0.2, count, Particle::Fishing);
        self.bite_countdown
            .store(rand::random_range(20..=40), Relaxed);
        self.entity
            .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, true);
        // FishingHook.onSyncedDataUpdated uses the UUID/game-time seeded random for the dip.
        // Mth.nextFloat subtracts the f32 endpoints before multiplying the draw.
        velocity.y = f64::from(
            -0.4f32 * (self.synced_random(world).next_f32() * (1.0f32 - 0.6f32) + 0.6f32),
        );
    }

    fn approach_particles(&self, world: &World, hooked: i32) {
        let angle = self.fish_angle.load() + rand::rng().triangle(0.0, 9.188) as f32;
        self.fish_angle.store(angle);
        let radians = angle.to_radians();
        let (sin, cos) = (
            pumpkin_util::math::sin(radians),
            pumpkin_util::math::cos(radians),
        );
        let pos = self.entity.pos.load();
        let fish = Vector3::new(
            pos.x + f64::from(sin * hooked as f32 * 0.1),
            pos.y.floor() + 1.0,
            pos.z + f64::from(cos * hooked as f32 * 0.1),
        );
        if world.get_block(&BlockPos::floored_v(fish.add_raw(0.0, -1.0, 0.0))) != &Block::WATER {
            return;
        }
        if rand::random::<f32>() < 0.15 {
            world.spawn_particle(
                fish.add_raw(0.0, -f64::from(0.1f32), 0.0),
                Vector3::new(sin, 0.1, cos),
                0.0,
                1,
                Particle::Bubble,
            );
        }
        let (dx, dz) = (sin * 0.04, cos * 0.04);
        world.spawn_particle(fish, Vector3::new(dz, 0.01, -dx), 1.0, 0, Particle::Fishing);
        world.spawn_particle(fish, Vector3::new(-dz, 0.01, dx), 1.0, 0, Particle::Fishing);
    }

    fn tease_particles(&self, world: &World, wait: i32) {
        let mut chance = 0.15;
        if wait < 20 {
            chance += (20 - wait) as f32 * 0.05;
        } else if wait < 40 {
            chance += (40 - wait) as f32 * 0.02;
        } else if wait < 60 {
            chance += (60 - wait) as f32 * 0.01;
        }
        if rand::random::<f32>() >= chance {
            return;
        }
        let angle = (rand::random::<f32>() * 360.0).to_radians();
        let distance = rand::random::<f32>().mul_add(35.0, 25.0);
        let pos = self.entity.pos.load();
        let fish = Vector3::new(
            pos.x + f64::from(pumpkin_util::math::sin(angle) * distance) * 0.1,
            pos.y.floor() + 1.0,
            pos.z + f64::from(pumpkin_util::math::cos(angle) * distance) * 0.1,
        );
        if world.get_block(&BlockPos::floored_v(fish.add_raw(0.0, -1.0, 0.0))) == &Block::WATER {
            world.spawn_particle(
                fish,
                Vector3::new(0.1, 0.0, 0.1),
                0.0,
                2 + rand::random_range(0..2),
                Particle::Splash,
            );
        }
    }
}
