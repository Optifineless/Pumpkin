//! `AbstractSkeleton.reassessWeaponGoal`, including offhand bows and species intervals.

use crate::entity::ai::goal::Goal;
use crate::entity::{
    ai::goal::{bow_attack::BowAttackGoal, melee_attack::MeleeAttackGoal},
    living::LivingEntity,
    mob::Mob,
};
use pumpkin_data::{data_component_impl::EquipmentSlot, entity::EntityType, item::Item};
use pumpkin_inventory::entity_equipment::EntityEquipment;
use pumpkin_util::Difficulty;
use pumpkin_util::Hand;
use std::sync::atomic::Ordering::Relaxed;

pub fn equipment_changed(living: &LivingEntity) {
    if !is_skeleton(living.entity.entity_type) {
        return;
    }
    if let Some(entity) = living
        .entity
        .world
        .load()
        .get_entity_by_id(living.entity.entity_id)
        && let Some(mob) = entity.get_mob()
    {
        reassess_weapon_goal(mob);
    }
}

pub fn reassess_weapon_goal(mob: &dyn Mob) {
    mob.get_mob_entity().weapon_goal_dirty.store(true, Relaxed);
    reassess_if_dirty(mob);
}

pub fn reassess_if_dirty(mob: &dyn Mob) {
    let base = mob.get_mob_entity();
    if !is_skeleton(mob.get_entity().entity_type) {
        return;
    }
    if !base.weapon_goal_dirty.swap(false, Relaxed) {
        return;
    }
    let difficulty = mob.get_entity().world.load().level_info.load().difficulty;
    let interval = {
        // Equipment callers may still hold their equipment lock; retry before AI next tick.
        let Ok(equipment) = base.living_entity.entity_equipment.try_lock() else {
            base.weapon_goal_dirty.store(true, Relaxed);
            return;
        };
        weapon_interval(&equipment, mob.get_entity().entity_type, difficulty)
    };
    let stopped = {
        // onEquipItem can run inside a goal. Never re-enter the goal-selector mutex.
        let Ok(mut goals) = base.goals_selector.try_lock() else {
            base.weapon_goal_dirty.store(true, Relaxed);
            return;
        };
        let mut stopped = goals.remove_goals::<BowAttackGoal>();
        stopped.extend(goals.remove_goals::<MeleeAttackGoal>());
        if let Some(interval) = interval {
            goals.add_goal(4, Box::new(BowAttackGoal::new(1.0, interval, 15.0)));
        } else {
            goals.add_goal(4, Box::new(MeleeAttackGoal::new(1.2, false)));
        }
        stopped
    };
    for mut goal in stopped {
        goal.stop(mob);
    }
}

/// Selects the bow hand as `ProjectileUtil.getWeaponHoldingHand` does, preferring the main hand.
pub fn bow_hand(equipment: &EntityEquipment) -> Option<Hand> {
    if equipment.get(&EquipmentSlot::MAIN_HAND).item == &Item::BOW {
        Some(Hand::Right)
    } else if equipment.get(&EquipmentSlot::OFF_HAND).item == &Item::BOW {
        Some(Hand::Left)
    } else {
        None
    }
}

fn weapon_interval(
    equipment: &EntityEquipment,
    ty: &EntityType,
    difficulty: Difficulty,
) -> Option<i32> {
    bow_hand(equipment).map(|_| attack_interval(ty, difficulty))
}

fn attack_interval(ty: &EntityType, difficulty: Difficulty) -> i32 {
    // AbstractSkeleton.getHardAttackInterval/getAttackInterval; Bogged and Parched override both.
    match (ty.resource_name, difficulty) {
        ("bogged" | "parched", Difficulty::Hard) => 50,
        ("bogged" | "parched", _) => 70,
        (_, Difficulty::Hard) => 20,
        _ => 40,
    }
}

fn is_skeleton(ty: &EntityType) -> bool {
    matches!(
        ty.resource_name,
        "skeleton" | "stray" | "bogged" | "parched" | "wither_skeleton"
    )
}
