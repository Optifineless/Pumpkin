use crate::entity::ai::{
    control::guardian_move_control::GuardianMoveControl, pathfinder::Navigator,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use pumpkin_data::{damage::DamageType, sound::Sound};

use crate::entity::custom_sound::CustomSound;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct GuardianEntity {
    pub mob_entity: MobEntity,
}

impl GuardianEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Guardian constructor / createNavigation, inherited by ElderGuardian.
        let navigation = Navigator::water_bound(false);
        mob_entity.configure_movement(navigation, GuardianMoveControl::default());
        let guardian = Self { mob_entity };
        let mob_arc = Arc::new(guardian);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(4, Box::new(WanderAroundGoal::swimming(1.0, 80)));
            goal_selector.add_goal(
                5,
                LookAtEntityGoal::with_default(mob_weak.clone(), &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::SQUID, true),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::GLOW_SQUID, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::AXOLOTL, true),
            );
        };

        mob_arc
    }
}

impl CustomSound for GuardianEntity {
    fn hurt_sound(&self) -> Option<Sound> {
        let entity = self.get_entity();
        let is_water = entity.touching_water.load(Ordering::Relaxed);
        Some(if is_water {
            Sound::EntityGuardianHurt
        } else {
            Sound::EntityGuardianHurtLand
        })
    }

    fn death_sound(&self) -> Option<Sound> {
        let entity = self.get_entity();
        let is_water = entity.touching_water.load(Ordering::Relaxed);
        Some(if is_water {
            Sound::EntityGuardianDeath
        } else {
            Sound::EntityGuardianDeathLand
        })
    }
}

impl Mob for GuardianEntity {
    // Guardian.getMaxHeadXRot.
    fn get_max_look_pitch_change(&self) -> f32 {
        180.0
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        super::movement::flop_on_land(self, 0.4);
    }

    // Guardian.travelInWater.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let sink = !self
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_moving()
            && self.mob_entity.get_target().is_none();
        crate::entity::mob::movement::travel_in_water(self, caller, 0.1, sink)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn on_damage(
        &self,
        _damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        // Guardian.hurtServer checks the direct entity for thorns, never the shooter.
        if let Some(src) = source
            && let Some(living) = src.get_living_entity()
        {
            let _ = living.damage(src, 2.0, DamageType::THORNS);
        }
    }
}
