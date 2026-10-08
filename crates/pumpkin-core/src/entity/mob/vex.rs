use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering::Relaxed},
};

use pumpkin_data::entity::EntityType;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, swim::SwimGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct VexEntity {
    pub mob_entity: MobEntity,
    charging: AtomicBool,
}

impl VexEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vex constructor / tick: free flight uses its own acceleration control.
        mob_entity.living_entity.entity.set_has_no_gravity(true);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(crate::entity::ai::control::vex_move_control::VexMoveControl::default());
        let vex = Self {
            mob_entity,
            charging: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(vex);
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

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            // Vex.registerGoals: charges bypass ground navigation.
            goal_selector.add_goal(4, Box::new(super::vex_movement::VexChargeAttackGoal));
            goal_selector.add_goal(8, Box::new(super::vex_movement::VexRandomMoveGoal));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak.clone(), &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));

            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
        };

        mob_arc
    }

    pub(super) fn is_charging(&self) -> bool {
        self.charging.load(Relaxed)
    }

    pub(super) fn set_charging(&self, charging: bool) {
        // Vex.setIsCharging / FLAG_IS_CHARGING.
        self.charging.store(charging, Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::vex::DATA_FLAGS_ID,
            i8::from(charging),
        );
    }
}

impl Mob for VexEntity {
    // Vex.tick enables noPhysics only across the superclass tick, including travel.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.get_entity()
            .no_physics
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    fn post_tick(&self) {
        self.get_entity()
            .no_physics
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn get_mob_gravity(&self) -> f64 {
        0.0
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }
}
