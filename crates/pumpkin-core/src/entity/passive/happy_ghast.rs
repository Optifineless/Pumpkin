use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::Animal,
    player::Player,
};

#[path = "happy_ghast_flight.rs"]
mod flight;
#[path = "happy_ghast_goals.rs"]
mod goals;
#[path = "happy_ghast_home.rs"]
mod home;

pub const HAPPY_GHAST_FOOD: &[&Item] = &[&Item::SNOWBALL];

/// Represents a Happy Ghast, a passive flying mob.
///
/// Wiki: <https://minecraft.wiki/w/Happy_Ghast>
pub struct HappyGhastEntity {
    pub mob_entity: MobEntity,
    weak_self: Weak<Self>,
    flight_enabled: bool,
    adult_flight: AtomicBool,
    home: crossbeam::atomic::AtomicCell<home::Home>,
    facing_target: crossbeam::atomic::AtomicCell<Option<pumpkin_util::math::vector3::Vector3<f64>>>,
    #[cfg(test)]
    pub(crate) random_draws: std::sync::Mutex<std::collections::VecDeque<f64>>,
    pub ageable_data: AgeableData,
    pub server_still_timeout: AtomicI32,
    pub leash_holder_time: AtomicI32,
    pub is_leash_holder: AtomicBool,
    pub stays_still: AtomicBool,
}

impl HappyGhastEntity {
    #[cfg(test)]
    pub(crate) fn floating_goal_count(&self) -> usize {
        self.mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .goal_count::<goals::RandomFloatAroundGoal>()
    }

    #[cfg(test)]
    pub(crate) fn temptation_running(&self) -> bool {
        self.mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_goal_running::<goals::TemptGoal>()
    }

    pub fn new(entity: Entity) -> Arc<Self> {
        let flight_enabled = entity
            .world
            .load()
            .server
            .upgrade()
            .is_none_or(|server| server.advanced_config.happy_ghast.enabled);
        let mob_entity = MobEntity::new(entity);
        let mob_arc = Arc::new_cyclic(|weak| Self {
            weak_self: weak.clone(),
            flight_enabled,
            adult_flight: AtomicBool::new(false),
            home: crossbeam::atomic::AtomicCell::new(home::Home::default()),
            facing_target: crossbeam::atomic::AtomicCell::new(None),
            #[cfg(test)]
            random_draws: std::sync::Mutex::default(),
            mob_entity,
            ageable_data: AgeableData::default(),
            server_still_timeout: AtomicI32::new(0),
            leash_holder_time: AtomicI32::new(0),
            is_leash_holder: AtomicBool::new(false),
            stays_still: AtomicBool::new(false),
        });
        mob_arc.install_legacy_goals();
        mob_arc.reconcile_flight();
        mob_arc
    }

    fn install_legacy_goals(&self) {
        let Some(mob_arc) = self.weak_self.upgrade() else {
            return;
        };
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc;
            Arc::downgrade(&mob_arc)
        };
        {
            let mut goal_selector = self
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(
                1,
                Box::new(TemptGoal::with_stop_distance(
                    1.0,
                    HAPPY_GHAST_FOOD,
                    false,
                    7.0,
                )),
            );
            goal_selector.add_goal(2, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                3,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(4, Box::new(RandomLookAroundGoal::default()));
        };
    }

    pub fn set_server_still_timeout(&self, timeout: i32) {
        self.server_still_timeout.store(timeout, Ordering::Relaxed);
        self.sync_stay_still_flag();
    }

    #[must_use]
    pub fn is_on_still_timeout(&self) -> bool {
        self.stays_still.load(Ordering::Relaxed)
            || self.server_still_timeout.load(Ordering::Relaxed) > 0
    }

    fn set_leash_holder(&self, is_leash_holder: bool) {
        self.is_leash_holder
            .store(is_leash_holder, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::IS_LEASH_HOLDER,
            is_leash_holder,
        );
    }

    fn sync_stay_still_flag(&self) {
        let stays_still = self.server_still_timeout.load(Ordering::Relaxed) > 0;
        self.stays_still.store(stays_still, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::STAYS_STILL,
            stays_still,
        );
    }
}

impl AgeableMob for HappyGhastEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for HappyGhastEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack
            .item
            .has_tag(&tag::Item::MINECRAFT_HAPPY_GHAST_FOOD)
            || HAPPY_GHAST_FOOD.iter().any(|i| i.id == item_stack.item.id)
    }
}

impl Mob for HappyGhastEntity {
    // HappyGhastBodyRotationControl.clientTick, then the ordinary body controller.
    fn custom_body_rotation(&self) -> bool {
        let entity = self.get_entity();
        if entity.has_passengers() {
            entity.head_yaw.store(entity.yaw.load());
            entity.body_yaw.store(entity.head_yaw.load());
        }
        false
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.home.load().write(nbt);
        nbt.put_int(
            "still_timeout",
            self.server_still_timeout.load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.home.store(home::Home::read(nbt));
        self.set_server_still_timeout(nbt.get_int("still_timeout").unwrap_or(0));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();

        let leash_time = self.leash_holder_time.load(Ordering::Relaxed);
        if leash_time > 0 {
            self.leash_holder_time.fetch_sub(1, Ordering::Relaxed);
        }
        self.set_leash_holder(leash_time > 0);

        let still_timeout = self.server_still_timeout.load(Ordering::Relaxed);
        if still_timeout > 0 {
            let entity = self.get_entity();
            if entity.age.load(Ordering::Relaxed) > 60 {
                self.server_still_timeout.fetch_sub(1, Ordering::Relaxed);
            }
            self.sync_stay_still_flag();
        }

        // Continuous healing
        let entity = self.get_entity();
        if entity.is_alive() {
            let living = &self.mob_entity.living_entity;
            let _owner = living.own_damage();
            let current_health = living.health.load();
            let max_health = living.get_max_health();
            if current_health < max_health {
                let world = entity.world.load();
                let ticks = world.get_world_age();
                let heal_interval = 600;
                if ticks % heal_interval == 0 {
                    living.heal(1.0);
                }
            }
        }
    }

    fn after_base_tick(&self) {
        self.reconcile_flight();
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if self.uses_adult_flight() {
            self.facing_target.store(
                self.mob_entity
                    .get_target()
                    .map(|target| target.get_entity().pos.load()),
            );
            self.check_restriction();
            if self.is_on_still_timeout() {
                // Release navigation before the runner takes MoveControl; no leash momentum exists yet.
                self.mob_entity
                    .navigator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .stop();
            }
        }
    }

    fn can_tick_navigation(&self) -> bool {
        !self.uses_adult_flight()
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        if !self.uses_adult_flight() {
            return false;
        }
        let speed = self
            .mob_entity
            .living_entity
            .get_attribute_value(&pumpkin_data::attributes::Attributes::FLYING_SPEED)
            as f32
            * 5.0
            / 3.0;
        crate::entity::mob::movement::travel_flying(self, caller, speed, speed, speed)
    }

    fn check_fall_damage(&self) -> bool {
        self.uses_adult_flight()
    }

    fn mob_on_climbable(&self) -> bool {
        false
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::happy_ghast::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::IS_LEASH_HOLDER,
            self.is_leash_holder.load(Ordering::Relaxed),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::STAYS_STILL,
            self.stays_still.load(Ordering::Relaxed),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if self.is_baby() {
            return self.animal_interact(player, item_stack, Sound::EntityGhastlingAmbient);
        }

        self.animal_interact(player, item_stack, Sound::EntityHappyGhastAmbient)
    }
}
