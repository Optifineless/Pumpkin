mod award;
mod collection;
mod movement;
pub use collection::collect_nearby_orbs;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;

use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicI32, Ordering},
};

use pumpkin_data::{damage::DamageType, tag::Taggable, tracked_data};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_protocol::VarInt;
use pumpkin_util::math::vector3::Vector3;

use crate::server::Server;

use super::{Entity, EntityBase, player::Player};

// ExperienceOrb's Java constants; 40 is the number of ID groups, not a count limit.
const LIFETIME: i32 = 6000;
const ENTITY_SCAN_PERIOD: i32 = 20;
const MAX_FOLLOW_DIST: f64 = 8.0;
const ORB_GROUPS_PER_AREA: i32 = 40;
const ORB_MERGE_DISTANCE: f64 = 0.5;
const DEFAULT_HEALTH: i32 = 5;
const DEFAULT_COUNT: i32 = 1;

struct OrbState {
    health: i32,
    age: i32,
    count: i32,
}

pub struct ExperienceOrbEntity {
    entity: Entity,
    value: AtomicI32,
    // Vanilla ticks serially. Keep count/age transfers atomic across Pumpkin's parallel ticks.
    state: Mutex<OrbState>,
    following_player: Mutex<Option<Weak<Player>>>,
}

impl ExperienceOrbEntity {
    /// Constructs a summon/load orb with `ExperienceOrb(EntityType, Level)` defaults and no launch.
    /// Factory callers use this before applying NBT; XP drops use [`Self::award`] instead.
    pub fn new_empty(entity: Entity) -> Self {
        Self {
            entity,
            value: AtomicI32::new(0),
            state: Mutex::new(OrbState {
                health: DEFAULT_HEALTH,
                age: 0,
                count: DEFAULT_COUNT,
            }),
            following_player: Mutex::default(),
        }
    }

    pub fn new(entity: Entity, amount: u32) -> Self {
        Self::new_with_direction(entity, Vector3::default(), amount, &mut rand::rng())
    }

    #[must_use]
    pub fn get_value(&self) -> i32 {
        self.value.load(Ordering::Relaxed)
    }

    fn set_value(&self, value: i32) {
        self.value.store(value, Ordering::Relaxed);
        self.entity
            .set_synced_data(tracked_data::experience_orb::DATA_VALUE, VarInt(value));
    }

    // ExperienceOrb.canMerge / merge. Lock in ID order so reciprocal scans cannot deadlock.
    fn merge(&self, other: &Self) {
        if self.entity.entity_id == other.entity.entity_id
            || !other.can_merge(self.entity.entity_id, self.get_value())
        {
            return;
        }
        let (first, second) = if self.entity.entity_id < other.entity.entity_id {
            (&self.state, &other.state)
        } else {
            (&other.state, &self.state)
        };
        let mut first = first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut second = second
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (ours, theirs) = if self.entity.entity_id < other.entity.entity_id {
            (&mut first, &mut second)
        } else {
            (&mut second, &mut first)
        };
        if ours.count == 0
            || theirs.count == 0
            || self.entity.is_removed()
            || other.entity.is_removed()
        {
            return;
        }
        let Some(count) = ours.count.checked_add(theirs.count) else {
            return;
        };
        ours.count = count;
        ours.age = ours.age.min(theirs.age);
        theirs.count = 0;
        drop(first);
        drop(second);
        other.entity.remove();
    }

    fn can_merge(&self, id: i32, value: i32) -> bool {
        // ExperienceOrb.canMerge uses Java's wrapping int subtraction before modulo.
        !self.entity.is_removed()
            && self.entity.entity_id.wrapping_sub(id) % ORB_GROUPS_PER_AREA == 0
            && self.get_value() == value
    }

    fn scan_for_merges(&self) {
        let area = self
            .entity
            .bounding_box
            .load()
            .expand_all(ORB_MERGE_DISTANCE);
        for entity in self.entity.world.load().experience_orbs().iter() {
            if let Some(orb) = entity.cast_any().downcast_ref::<Self>()
                && orb.entity.bounding_box.load().intersects(&area)
            {
                self.merge(orb);
            }
        }
    }

    // ExperienceOrb.followNearbyPlayer: attract toward half the player's eye height.
    fn follow_nearby_player(&self) -> bool {
        let mut following = self
            .following_player
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pos = self.entity.pos.load();
        let mut player = following.as_ref().and_then(Weak::upgrade);
        if player.as_ref().is_none_or(|player| {
            player.is_spectator()
                || player.get_entity().pos.load().sub(&pos).length_squared()
                    > MAX_FOLLOW_DIST.powi(2)
        }) {
            player = self
                .entity
                .world
                .load()
                .players
                .load()
                .iter()
                .filter(|player| !player.is_spectator())
                .filter_map(|player| {
                    let distance = player.get_entity().pos.load().sub(&pos).length_squared();
                    (distance < MAX_FOLLOW_DIST.powi(2)).then_some((distance, player.clone()))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, player)| player)
                .filter(|player| player.can_collect_experience());
            *following = player.as_ref().map(Arc::downgrade);
        }
        drop(following);
        player.is_some_and(|player| {
            let player_entity = player.get_entity();
            let target =
                player_entity
                    .pos
                    .load()
                    .add_raw(0.0, player_entity.get_eye_height() / 2.0, 0.0);
            let delta = target.sub(&pos);
            let length = delta.length();
            let power = 1.0 - length / MAX_FOLLOW_DIST;
            // Vec3.normalize uses the float threshold 1.0E-5F promoted to double.
            let direction = if length < f64::from(1.0e-5f32) {
                Vector3::default()
            } else {
                delta * (1.0 / length)
            };
            self.entity
                .velocity
                .store(self.entity.velocity.load() + direction * (power * power * 0.1));
            true
        })
    }

    fn tick_age(&self) {
        let expired = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.age += 1;
            state.age >= LIFETIME
        };
        if expired {
            self.entity.remove();
        }
    }
}

impl EntityBase for ExperienceOrbEntity {
    // ExperienceOrb.tick, including setUnderwaterMovement / getAirDrag.
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = &self.entity;
        entity.tick(caller, server);
        if entity.is_removed() {
            return;
        }
        let world = entity.world.load_full();
        let colliding = !world.is_space_empty(entity.bounding_box.load());
        let mut velocity = entity.velocity.load();
        if entity.is_submerged_in_water() {
            velocity = Self::underwater_movement(velocity);
        } else if !colliding && !entity.has_no_gravity() {
            velocity.y -= self.get_gravity();
        }
        if world
            .get_fluid(&entity.block_pos.load())
            .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_LAVA)
        {
            velocity = Vector3::new(
                f64::from((rand::random::<f32>() - rand::random::<f32>()) * 0.2),
                f64::from(0.2f32),
                f64::from((rand::random::<f32>() - rand::random::<f32>()) * 0.2),
            );
        }
        entity.velocity.store(velocity);
        if entity.age.load(Ordering::Relaxed) % ENTITY_SCAN_PERIOD == 1 {
            self.scan_for_merges();
        }
        if entity.is_removed() {
            return;
        }
        if !self.follow_nearby_player()
            && colliding
            && !world.is_space_empty(entity.bounding_box.load().shift(entity.velocity.load()))
        {
            let pos = entity.pos.load();
            let bb = entity.bounding_box.load();
            entity.move_towards_closest_space(Vector3::new(
                pos.x,
                f64::midpoint(bb.min.y, bb.max.y),
                pos.z,
            ));
            entity.velocity_dirty.store(true, Ordering::Relaxed);
        }
        let fall_speed = entity.velocity.load().y;
        entity.move_entity(caller, entity.velocity.load());
        entity.tick_block_collisions(caller);
        let mut friction = 0.98f32;
        if entity.on_ground.load(Ordering::Relaxed) {
            let below = self.get_block_pos_below_that_affects_my_movement();
            friction *= world.get_block(&below).slipperiness;
        }
        velocity = entity.velocity.load() * f64::from(friction);
        let gravity = if entity.has_no_gravity() {
            0.0
        } else {
            self.get_gravity()
        };
        if entity.vertical_collision.load(Ordering::Relaxed) && fall_speed < -gravity {
            velocity.y = -fall_speed * 0.4;
        }
        entity.velocity.store(velocity);
        self.tick_age();
    }

    fn init_data_tracker(&self) {
        self.set_value(self.get_value());
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&super::living::LivingEntity> {
        None
    }

    // ExperienceOrb.playerTouch; consume one represented orb, not the whole cluster.
    fn on_player_collision(&self, player: &Arc<Player>) {
        if !player.can_collect_experience() || self.entity.is_removed() {
            return;
        }
        let empty = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.count == 0 || !player.try_take_experience() {
                return;
            }
            state.count -= 1;
            state.count == 0
        };
        if !player.living_entity.pickup(&self.entity, 1) {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .count += 1;
            return;
        }
        let remaining = player.apply_mending_from_xp(self.get_value());
        if remaining > 0 {
            player.add_experience_points(remaining);
        }
        if empty {
            self.entity.remove();
        }
    }

    // ExperienceOrb.hurtServer uses Entity.isInvulnerableToBase, including damage tags.
    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        damage: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.entity.is_invulnerable_to(&damage_type, cause) {
            return false;
        }
        self.entity.velocity_dirty.store(true, Ordering::Relaxed);
        let dead = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.health = (state.health as f32 - damage) as i32;
            state.health <= 0
        };
        if dead {
            self.entity.remove();
        }
        true
    }

    // ExperienceOrb.addAdditionalSaveData / readAdditionalSaveData use shorts and a positive Count.
    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        nbt.put_short("Health", state.health as i16);
        nbt.put_short("Age", state.age as i16);
        nbt.put_short("Value", self.get_value() as i16);
        nbt.put_int("Count", state.count);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        let state = OrbState {
            health: i32::from(
                nbt.get_numeric_short("Health")
                    .unwrap_or(DEFAULT_HEALTH as i16),
            ),
            age: i32::from(nbt.get_numeric_short("Age").unwrap_or(0)),
            // ExtraCodecs.POSITIVE_INT reads Number.intValue, including other numeric tag types.
            count: nbt
                .get("Count")
                .and_then(|tag| match *tag {
                    NbtTag::Byte(value) => Some(i32::from(value)),
                    NbtTag::Short(value) => Some(i32::from(value)),
                    NbtTag::Int(value) => Some(value),
                    NbtTag::Long(value) => Some(value as i32),
                    NbtTag::Float(value) => Some(value as i32),
                    NbtTag::Double(value) => Some(value as i32),
                    _ => None,
                })
                .filter(|count| *count > 0)
                .unwrap_or(DEFAULT_COUNT),
        };
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
        self.set_value(i32::from(nbt.get_numeric_short("Value").unwrap_or(0)));
    }

    fn get_gravity(&self) -> f64 {
        0.03
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
