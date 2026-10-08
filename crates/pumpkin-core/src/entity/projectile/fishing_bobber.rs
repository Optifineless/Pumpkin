use crate::{
    entity::{Entity, EntityBase, ai::util::RandomExt, living::LivingEntity, player::Player},
    server::Server,
    world::World,
};
use crossbeam::atomic::AtomicCell;
use pumpkin_data::{fluid::Fluid, item::Item, tracked_data};
use pumpkin_util::{
    Hand,
    math::vector3::Vector3,
    random::{RandomImpl, legacy_rand::LegacyRand},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI32, Ordering::Relaxed},
};

mod catching_fish;
mod collision;
#[cfg(test)]
mod event_tests;
mod open_water;
#[cfg(test)]
mod regression_tests;
mod retrieve;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;

// FishingHook's movement states; hooking switches from FLYING on the following tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HookState {
    Flying,
    HookedInEntity,
    Bobbing,
}

pub struct FishingBobberEntity {
    pub entity: Entity,
    pub projectile: super::ownership::ProjectileState,
    state: AtomicCell<HookState>,
    pub hooked_entity_id: AtomicI32,
    life: AtomicI32,
    tick_count: AtomicI32,
    pub wait_countdown: AtomicI32,
    pub bite_countdown: AtomicI32,
    hook_countdown: AtomicI32,
    fish_angle: AtomicCell<f32>,
    open_water: AtomicBool,
    out_of_water_time: AtomicI32,
    luck: i32,
    lure_speed: i32,
    hand: Hand,
}

impl FishingBobberEntity {
    const INERTIA: f64 = 0.92;
    // FishingHook.GRAVITY and MAX_OUT_OF_WATER_TIME.
    const GRAVITY: f64 = 0.03f32 as f64;
    const MAX_OUT_OF_WATER_TIME: i32 = 10;

    // FishingHook(Player, Level, int, int), using the use packet's rotation.
    pub fn new_with_rotation(
        entity: Entity,
        owner: &Player,
        yaw: f32,
        pitch: f32,
        luck: i32,
        lure_speed: i32,
        hand: Hand,
    ) -> Self {
        let mut rng = rand::rng();
        let spread = Vector3::new(
            rng.triangle(0.5, 0.010_336_5),
            rng.triangle(0.5, 0.010_336_5),
            rng.triangle(0.5, 0.010_336_5),
        );
        let (offset, velocity) = throw_setup(yaw, pitch, spread);
        entity.set_pos(
            owner.position() + offset.add_raw(0.0, owner.get_entity().get_eye_height(), 0.0),
        );
        entity.update_last_pos();
        entity.set_rotation(
            (velocity.x.atan2(velocity.z) as f32).to_degrees(),
            (velocity.y.atan2(velocity.horizontal_length()) as f32).to_degrees(),
        );
        entity.velocity.store(velocity);
        // FishingHook.getAddEntityPacket/recreateFromPacket: the client requires the caster id.
        entity.data.store(owner.get_entity().entity_id, Relaxed);
        Self {
            entity,
            projectile: super::ownership::ProjectileState::new(Some(
                owner.get_entity().entity_uuid,
            )),
            state: AtomicCell::new(HookState::Flying),
            hooked_entity_id: AtomicI32::new(-1),
            life: AtomicI32::new(0),
            tick_count: AtomicI32::new(0),
            wait_countdown: AtomicI32::new(0),
            bite_countdown: AtomicI32::new(0),
            hook_countdown: AtomicI32::new(0),
            fish_angle: AtomicCell::new(0.0),
            open_water: AtomicBool::new(true),
            out_of_water_time: AtomicI32::new(0),
            luck: luck.max(0),
            lure_speed: lure_speed.max(0),
            hand,
        }
    }

    // Keep the merged base-tick fixture's caster nearby with the rod FishingHook requires.
    #[cfg(test)]
    pub(super) fn new(entity: Entity, owner: &Player) -> Self {
        owner.get_entity().set_pos(entity.pos.load());
        owner.inventory().set_stack_in_hand(
            Hand::Right,
            pumpkin_data::item_stack::ItemStack::new(1, &Item::FISHING_ROD),
        );
        let (yaw, pitch) = owner.rotation();
        Self::new_with_rotation(entity, owner, yaw, pitch, 0, 0, Hand::Right)
    }

    // FishingHook.getPlayerOwner narrows Projectile.getOwner to a player.
    pub(super) fn get_player_owner(&self) -> Option<Arc<Player>> {
        Arc::downcast::<Player>(self.projectile_owner()?).ok()
    }

    /// Returns the hook's latched open-water state used by fishing treasure predicates.
    #[must_use]
    pub fn is_open_water_fishing(&self) -> bool {
        self.open_water.load(Relaxed)
    }

    pub(crate) fn fire_cast_event(&self, hand: Hand) -> Option<i32> {
        self.fire_fish_event(
            crate::plugin::api::events::player::fish::PlayerFishState::Fishing,
            None,
            hand,
            0,
        )
    }

    /// Clears the caster's reference after rejected spawning or external removal of this hook.
    pub fn clear_owner(&self) {
        if let Some(owner) = self.get_player_owner() {
            let _ =
                owner
                    .fishing_bobber
                    .compare_exchange(self.entity.entity_id, -1, Relaxed, Relaxed);
        }
    }

    // FishingHook.shouldStopFishing accepts a rod in either hand and exactly 32 blocks.
    pub(super) fn should_stop_fishing(&self, owner: &Player) -> bool {
        let inventory = owner.inventory();
        !can_interact_with_level(owner)
            || (inventory.held_item().item != &Item::FISHING_ROD
                && inventory.off_hand_item().item != &Item::FISHING_ROD)
            || self
                .entity
                .pos
                .load()
                .squared_distance_to_vec(&owner.position())
                > 1024.0
            || !std::sync::Arc::ptr_eq(&owner.world(), &self.entity.world.load_full())
    }

    // FishingHook.remove/updateOwnerInfo: don't clear a newer hook's owner reference.
    pub(super) fn discard(&self) {
        self.clear_owner();
        self.entity.remove();
    }

    pub(super) fn synced_random(&self, world: &World) -> LegacyRand {
        let (_, uuid_low) = self.entity.entity_uuid.as_u64_pair();
        let time = world
            .level_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .world_age;
        LegacyRand::from_seed(uuid_low ^ time as u64)
    }

    // FishingHook.tick: state transitions precede gravity, move, then inertia.
    pub fn process_tick(&self, caller: &dyn EntityBase) {
        self.tick_count.fetch_add(1, Relaxed);
        let Some(owner) = self.get_player_owner() else {
            self.discard();
            return;
        };
        if self.should_stop_fishing(&owner) {
            self.discard();
            return;
        }
        let entity = &self.entity;
        let world = entity.world.load();
        if entity.on_ground.load(Relaxed) {
            if self.life.fetch_add(1, Relaxed) + 1 >= 1200 {
                self.discard();
                return;
            }
        } else {
            self.life.store(0, Relaxed);
        }
        let pos = entity.block_pos.load();
        let (fluid, fluid_state) = world.get_fluid_and_fluid_state(&pos);
        let height = if fluid.matches_type(&Fluid::WATER) {
            f64::from(world.get_fluid_height(&pos, fluid, &fluid_state))
        } else {
            0.0
        };
        let in_water = height > 0.0;
        let mut velocity = entity.velocity.load();
        match self.state.load() {
            HookState::Flying => {
                if self.hooked_entity_id.load(Relaxed) != -1 {
                    entity.velocity.store(Vector3::default());
                    self.state.store(HookState::HookedInEntity);
                    return;
                }
                if in_water {
                    entity.velocity.store(velocity.multiply(0.3, 0.2, 0.3));
                    self.state.store(HookState::Bobbing);
                    return;
                }
                self.check_collision(&world, &mut velocity);
            }
            HookState::HookedInEntity => {
                self.follow_hooked_entity(&world);
                return;
            }
            HookState::Bobbing => {
                velocity = self.bob_tick(&world, &pos, height, velocity);
            }
        }
        if !in_water && !entity.on_ground.load(Relaxed) && self.hooked_entity_id.load(Relaxed) == -1
        {
            velocity.y -= Self::GRAVITY;
        }
        entity.velocity.store(velocity);
        // Entity.move clears these flags even for zero server-side movement.
        if velocity == Vector3::default() {
            entity.on_ground.store(false, Relaxed);
            entity.horizontal_collision.store(false, Relaxed);
        }
        entity.move_entity(caller, velocity);
        entity.tick_block_collisions(caller);
        super::arrow::update_flight_rotation(entity, entity.velocity.load(), true);
        if self.state.load() == HookState::Flying
            && (entity.on_ground.load(Relaxed) || entity.horizontal_collision.load(Relaxed))
        {
            entity.velocity.store(Vector3::default());
        }
        entity
            .velocity
            .store(entity.velocity.load() * Self::INERTIA);
    }

    fn bob_tick(
        &self,
        world: &World,
        pos: &pumpkin_util::math::position::BlockPos,
        height: f64,
        velocity: Vector3<f64>,
    ) -> Vector3<f64> {
        let mut velocity = bob_velocity(
            velocity,
            self.entity.pos.load().y,
            pos.0.y,
            height,
            rand::random::<f32>(),
        );
        if self.bite_countdown.load(Relaxed) <= 0 && self.hook_countdown.load(Relaxed) <= 0 {
            self.open_water.store(true, Relaxed);
        } else {
            self.open_water.store(
                self.open_water.load(Relaxed)
                    && self.out_of_water_time.load(Relaxed) < Self::MAX_OUT_OF_WATER_TIME
                    && self.calculate_open_water(pos),
                Relaxed,
            );
        }
        let out = self.out_of_water_time.load(Relaxed);
        if height > 0.0 {
            self.out_of_water_time.store((out - 1).max(0), Relaxed);
            if self.bite_countdown.load(Relaxed) > 0 {
                let mut random = self.synced_random(world);
                velocity.y -= 0.1 * f64::from(random.next_f32()) * f64::from(random.next_f32());
            }
            self.catching_fish(world, pos, &mut velocity);
        } else {
            self.out_of_water_time
                .store((out + 1).min(Self::MAX_OUT_OF_WATER_TIME), Relaxed);
        }
        velocity
    }
}

// Entity.canInteractWithLevel and LivingEntity.isAlive.
fn can_interact_with_level(entity: &dyn EntityBase) -> bool {
    entity.get_entity().is_alive()
        && !entity.is_spectator()
        && entity
            .get_living_entity()
            .is_none_or(|living| living.health.load() > 0.0)
}

// FishingHook.tick's water-surface spring; Java signum(0) is zero.
fn bob_velocity(
    velocity: Vector3<f64>,
    y: f64,
    block_y: i32,
    height: f64,
    random: f32,
) -> Vector3<f64> {
    let mut force = y + velocity.y - f64::from(block_y) - height;
    if force.abs() < 0.01 && force != 0.0 {
        force += force.signum() * 0.1;
    }
    Vector3::new(
        velocity.x * 0.9,
        velocity.y - force * f64::from(random) * 0.2,
        velocity.z * 0.9,
    )
}

// FishingHook's constructor scales each axis independently after clamping pitch.
fn throw_setup(yaw: f32, pitch: f32, triangle: Vector3<f64>) -> (Vector3<f64>, Vector3<f64>) {
    let yaw_rad = -yaw.to_radians() - std::f32::consts::PI;
    let pitch_rad = -pitch.to_radians();
    let (y_sin, y_cos) = (
        pumpkin_util::math::sin(yaw_rad),
        pumpkin_util::math::cos(yaw_rad),
    );
    let (x_sin, x_cos) = (
        pumpkin_util::math::sin(pitch_rad),
        pumpkin_util::math::cos(pitch_rad),
    );
    let offset = Vector3::new(-f64::from(y_sin) * 0.3, 0.0, -f64::from(y_cos) * 0.3);
    let movement = Vector3::new(
        -f64::from(y_sin),
        f64::from((x_sin / x_cos).clamp(-5.0, 5.0)),
        -f64::from(y_cos),
    );
    let scale = 0.6 / movement.length();
    (
        offset,
        movement.multiply(scale + triangle.x, scale + triangle.y, scale + triangle.z),
    )
}

impl EntityBase for FishingBobberEntity {
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.projectile)
    }
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn init_data_tracker(&self) {
        self.entity
            .set_synced_data(tracked_data::fishing_bobber::HOOKED_ENTITY, 0i32);
        self.entity
            .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, false);
    }
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        // FishingHook.canUsePortal is false; block contact may have queued a portal last tick.
        *self
            .entity
            .portal_manager
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.projectile.tick(&self.entity);
        // FishingHook.tick calls Projectile.tick -> Entity.tick before validating its owner.
        EntityBase::tick(&self.entity, caller, server);
        if self.entity.is_removed() {
            self.clear_owner();
        } else {
            self.process_tick(caller);
        }
    }
}

impl Drop for FishingBobberEntity {
    fn drop(&mut self) {
        self.clear_owner();
    }
}
