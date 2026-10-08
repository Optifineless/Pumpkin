mod spawn;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

use crate::entity::ai::util::goal_utils;
use crate::entity::{
    Entity, EntityBase,
    ai::behavior::neutral::apply_targets,
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal, revenge::RevengeGoal, wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        equipment::RegionalDifficulty,
        neutral::{NeutralData, NeutralMob},
    },
};
use crate::world::World;

/// Vanilla `ALERT_INTERVAL`: 4 to 6 seconds.
const ALERT_INTERVAL: std::ops::RangeInclusive<i32> = 80..=120;

pub struct ZombifiedPiglinEntity {
    pub mob_entity: MobEntity,
    neutral_data: NeutralData,
    ticks_until_next_alert: AtomicI32,
    /// Detects the target transition that restarts the alert interval.
    had_target: AtomicBool,
    is_baby: AtomicBool,
    can_break_doors: AtomicBool,
}

impl ZombifiedPiglinEntity {
    pub const XP_REWARD: u32 = 5;

    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let piglin = Self {
            mob_entity,
            neutral_data: NeutralData::default(),
            ticks_until_next_alert: AtomicI32::new(0),
            had_target: AtomicBool::new(false),
            is_baby: AtomicBool::new(false),
            can_break_doors: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(piglin);
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

            // ZombifiedPiglin.addBehaviourGoals does not register FloatGoal.
            goal_selector.add_goal(2, Box::new(MeleeAttackGoal::new(1.0, true)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(1.0)));
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
            target_selector.add_goal(1, Box::new(RevengeGoal::new(true).alerting_others()));
            apply_targets(&mut target_selector, &mob_arc.mob_entity, 2, 3, true);
        };

        mob_arc
    }

    /// Hands the current target to nearby piglins that have none of their own.
    fn maybe_alert_others(&self, target: &Arc<dyn EntityBase>) {
        let remaining = self.ticks_until_next_alert.load(Ordering::Relaxed);
        if remaining > 0 {
            self.ticks_until_next_alert
                .store(remaining - 1, Ordering::Relaxed);
            return;
        }

        if self.has_line_of_sight(target.get_entity()) {
            for other in goal_utils::nearby_same_type(self) {
                let Some(other_mob) = other.get_mob() else {
                    continue;
                };
                if other_mob.get_mob_entity().get_target().is_some()
                    || other.is_allied_to(target.as_ref())
                {
                    continue;
                }
                other_mob.set_mob_target(Some(target.clone()));
            }
        }

        self.ticks_until_next_alert
            .store(rand::random_range(ALERT_INTERVAL), Ordering::Relaxed);
    }
}

crate::impl_neutral_mob!(ZombifiedPiglinEntity, neutral_data);

impl Mob for ZombifiedPiglinEntity {
    fn finalize_spawn_with_context(
        &self,
        entity: &Arc<dyn EntityBase>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &RegionalDifficulty,
        reason: super::spawn::SpawnReason,
        group: Option<super::spawn::SpawnGroupData>,
    ) -> Option<super::spawn::SpawnGroupData> {
        Some(super::zombie::finalize::finalize_spawn(
            self,
            entity,
            view,
            difficulty,
            reason,
            group,
            |enabled| self.set_spawn_doors(enabled),
        ))
    }
    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.set_spawn_baby(nbt.get_bool("IsBaby").unwrap_or(false));
        self.set_spawn_doors(nbt.get_bool("CanBreakDoors").unwrap_or(false));
    }
    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        nbt.put_bool("IsBaby", self.is_baby.load(Ordering::Relaxed));
        nbt.put_bool(
            "CanBreakDoors",
            self.can_break_doors.load(Ordering::Relaxed),
        );
    }
    fn mob_init_data_tracker(&self) {
        self.set_spawn_baby(self.is_baby.load(Ordering::Relaxed));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_base_experience_reward(&self) -> u32 {
        // ZombifiedPiglin inherits Zombie.getBaseExperienceReward.
        let living = &self.mob_entity.living_entity;
        let base = living.entity.entity_type.experience_reward;
        let base = super::zombie::zombie_experience_base(
            base,
            living.entity.age.load(Ordering::Relaxed) < 0,
        );
        super::equipped_mob_experience(living, base)
    }

    fn spawn_as_baby(&self) -> bool {
        self.set_spawn_baby(true);
        true
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        let living = &self.mob_entity.living_entity;
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let weapon = if rand::random_range(0..20) == 0 {
            &Item::GOLDEN_SPEAR
        } else {
            &Item::GOLDEN_SWORD
        };
        equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, weapon));
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }

        let Some(target) = self.mob_entity.get_target() else {
            self.had_target.store(false, Ordering::Relaxed);
            return;
        };

        // Fresh target: wait out a full interval before spreading the word.
        if !self.had_target.swap(true, Ordering::Relaxed) {
            self.ticks_until_next_alert
                .store(rand::random_range(ALERT_INTERVAL), Ordering::Relaxed);
        }

        self.maybe_alert_others(&target);
    }
}
