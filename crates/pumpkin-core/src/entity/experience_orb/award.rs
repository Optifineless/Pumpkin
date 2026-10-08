use super::{ExperienceOrbEntity, ORB_GROUPS_PER_AREA};
use crate::{entity::Entity, world::World};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use rand::{Rng, RngExt};
use std::sync::Arc;

impl ExperienceOrbEntity {
    // ExperienceOrb(Level, Vec3, Vec3, int). Dimensions come from the generated EntityType.
    pub(super) fn new_with_direction(
        entity: Entity,
        direction: Vector3<f64>,
        amount: u32,
        random: &mut impl Rng,
    ) -> Self {
        let orb = Self::new_empty(entity);
        let entity = &orb.entity;
        entity.yaw.store(random.random::<f32>() * 360.0);
        let mut movement = Vector3::new(
            (random.random::<f64>() * 0.2 - 0.1) * 2.0,
            random.random::<f64>() * 0.2 * 2.0,
            (random.random::<f64>() * 0.2 - 0.1) * 2.0,
        );
        if direction.length_squared() > 0.0 && direction.dot(&movement) < 0.0 {
            movement = movement * -1.0;
        }
        let size = (f64::from(entity.width()) * 2.0 + f64::from(entity.height())) / 3.0;
        let length = direction.length();
        // Vec3.normalize uses 1.0E-5F, promoted from float to double.
        let direction = if length < f64::from(1.0e-5f32) {
            Vector3::default()
        } else {
            direction * (1.0 / length)
        };
        entity.set_pos(entity.pos.load() + direction * (size * 0.5));
        entity.velocity.store(movement);
        if !orb
            .entity
            .world
            .load()
            .is_space_empty(orb.entity.bounding_box.load())
        {
            orb.unstuck_if_possible(size);
        }
        orb.set_value(amount as i32);
        orb
    }

    /// Awards XP using vanilla denominations, award-time merging and random launch movement.
    /// Existing drop callers use this entry point, so plugins retain their spawn/drop events.
    pub fn spawn(world: &Arc<World>, position: Vector3<f64>, amount: u32) {
        Self::award(world, position, amount);
    }

    /// Spawns one raw-value orb with random launch and no splitting or award-time merging.
    /// Breeding, trading and `FishingHook.retrieve` use the direct `ExperienceOrb` constructor.
    pub fn spawn_single(world: &Arc<World>, position: Vector3<f64>, amount: u32) {
        let entity = Entity::new(world.clone(), position, &EntityType::EXPERIENCE_ORB);
        world.spawn_entity(Arc::new(Self::new_with_direction(
            entity,
            Vector3::default(),
            amount,
            &mut rand::rng(),
        )));
    }

    /// Mirrors `ExperienceOrb.award`; each represented orb is picked up separately.
    pub fn award(world: &Arc<World>, position: Vector3<f64>, amount: u32) {
        Self::award_with_direction(world, position, Vector3::default(), amount);
    }

    /// Mirrors `ExperienceOrb.awardWithDirection`, biasing launch movement toward the supplied direction.
    pub fn award_with_direction(
        world: &Arc<World>,
        position: Vector3<f64>,
        direction: Vector3<f64>,
        amount: u32,
    ) {
        Self::award_with_random(world, position, direction, amount, &mut rand::rng());
    }

    pub(super) fn award_with_random(
        world: &Arc<World>,
        position: Vector3<f64>,
        direction: Vector3<f64>,
        mut amount: u32,
        random: &mut impl Rng,
    ) {
        // ExperienceOrb.awardWithDirection / tryMergeToExisting.
        let area = BoundingBox::new(
            position.add_raw(-0.5, -0.5, -0.5),
            position.add_raw(0.5, 0.5, 0.5),
        );
        // Keep newly spawned denominations visible within this award, including during a world tick.
        let mut orbs: Vec<_> = world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.cast_any().is::<Self>())
            .cloned()
            .collect();
        while amount > 0 {
            let value = Self::get_experience_value(amount);
            amount -= value;
            let id = random.random_range(0..ORB_GROUPS_PER_AREA);
            let merged = orbs.iter().any(|entity| {
                let Some(orb) = entity.cast_any().downcast_ref::<Self>() else {
                    return false;
                };
                if !orb.entity.bounding_box.load().intersects(&area)
                    || !orb.can_merge(id, value as i32)
                {
                    return false;
                }
                let mut state = orb
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.count == 0 || orb.entity.is_removed() {
                    return false;
                }
                let Some(count) = state.count.checked_add(1) else {
                    return false;
                };
                state.count = count;
                state.age = 0;
                true
            });
            if !merged {
                let entity = Entity::new(world.clone(), position, &EntityType::EXPERIENCE_ORB);
                let orb: Arc<dyn crate::entity::EntityBase> =
                    Arc::new(Self::new_with_direction(entity, direction, value, random));
                if world.spawn_entity(orb.clone()) {
                    orbs.push(orb);
                }
            }
        }
    }

    // These denominations are hardcoded in ExperienceOrb.getExperienceValue.
    const fn get_experience_value(value: u32) -> u32 {
        if value >= 2477 {
            2477
        } else if value >= 1237 {
            1237
        } else if value >= 617 {
            617
        } else if value >= 307 {
            307
        } else if value >= 149 {
            149
        } else if value >= 73 {
            73
        } else if value >= 37 {
            37
        } else if value >= 17 {
            17
        } else if value >= 7 {
            7
        } else if value >= 3 {
            3
        } else {
            1
        }
    }
}
