use super::{Mob, create_ominous_banner};
use crate::entity::{EntityBase, ai::pathfinder::NavigatorGoal};
use pumpkin_data::data_component_impl::EquipmentSlot;
use rand::RngExt;
use std::sync::atomic::Ordering::Relaxed;

// Raider.ObtainRaidLeaderBannerGoal.cannotPickUpBanner.
fn can_pick_up_banner(mob: &dyn Mob) -> bool {
    let Some(raider) = mob.as_raider() else {
        return false;
    };
    if !raider.can_be_leader() {
        return false;
    }
    let living = &mob.get_mob_entity().living_entity;
    let head = {
        let Ok(equipment) = living.entity_equipment.try_lock() else {
            return false;
        };
        equipment.get(&EquipmentSlot::HEAD)
    };
    if head.are_equal(&create_ominous_banner()) {
        return false;
    }
    let Some(id) = raider.get_raider_data().raid_id.load() else {
        return false;
    };
    let world = mob.get_entity().world.load();
    let leader = {
        let raids = world
            .raids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(raid) = raids.get(id) else {
            return false;
        };
        if !raid.is_active() || raid.is_over() {
            return false;
        }
        raid.group_to_leader_map.get(&raider.get_wave()).copied()
    };
    !leader
        .and_then(|id| world.get_entity_by_uuid(id))
        .is_some_and(|e| {
            e.get_living_entity()
                .is_some_and(|l| !l.dead.load(Relaxed) && l.health.load() > 0.0)
        })
}

// Raider.ALLOWED_ITEMS: the entire stack, including count and components, must match.
fn allowed_item(entity: &dyn EntityBase) -> bool {
    let Some(item) = entity.get_item_entity() else {
        return false;
    };
    if !entity.get_entity().is_alive() || item.get_pickup_delay() != 0 {
        return false;
    }
    let stack = item
        .get_item_stack()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    stack.are_equal(&create_ominous_banner())
}

pub(super) fn can_start(mob: &dyn Mob) -> bool {
    can_pick_up_banner(mob) && nearby_banner(mob).is_some()
}

fn nearby_banner(mob: &dyn Mob) -> Option<std::sync::Arc<dyn EntityBase>> {
    let raw = mob.get_entity();
    raw.world
        .load()
        .get_entities_at_box(&raw.bounding_box.load().expand(16.0, 4.0, 16.0))
        .into_iter()
        .find(|e| allowed_item(e.as_ref()))
}

pub(super) fn tick(mob: &dyn Mob) {
    if !can_pick_up_banner(mob) {
        return;
    }
    let Some(candidate) = nearby_banner(mob) else {
        return;
    };
    let pos = mob.get_entity().pos.load();
    let destination = candidate.get_entity().pos.load();
    if pos.squared_distance_to_vec(&destination) < 2.0 {
        pick_up_banner(mob, candidate.as_ref());
    } else {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_progress(NavigatorGoal {
                current_progress: pos,
                destination,
                speed: 1.15,
            });
    }
}

fn pick_up_banner(mob: &dyn Mob, candidate: &dyn EntityBase) {
    let Some(raider) = mob.as_raider() else {
        return;
    };
    let Some(item) = candidate.get_item_entity() else {
        return;
    };
    let Some(stack) = item.take_for_banner() else {
        return;
    };
    // Validate owned stock without item, raid or equipment guards across component callbacks.
    if !stack.are_equal(&create_ominous_banner()) || !can_pick_up_banner(mob) {
        item.return_from_banner(stack);
        return;
    }
    let living = &mob.get_mob_entity().living_entity;
    let announcement = stack.clone();
    let previous_chance = living
        .equipment_drop_chances
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&EquipmentSlot::HEAD)
        .copied()
        .unwrap_or(super::super::equipment::DEFAULT_EQUIPMENT_DROP_CHANCE);
    let mut offered = Some(stack);
    let previous = equip_banner(mob, &mut offered);
    if let Some(returned) = offered {
        item.return_from_banner(returned);
        return;
    }
    let world = mob.get_entity().world.load_full();
    raider.set_patrol_leader(true);
    mob.get_mob_entity()
        .set_guaranteed_drop(&EquipmentSlot::HEAD);
    item.finish_transfer(item.stack_is_empty_between_transfers());
    // Raider.pickUpItem may drop displaced equipment before publishing the pickup.
    if let Some(previous) = previous
        && !previous.is_empty()
        && (rand::rng().random::<f32>() - 0.1).max(0.0) < previous_chance
    {
        let pos = mob.get_entity().pos.load();
        world.scatter_stack(pos.x, pos.y, pos.z, previous);
    }
    living.send_equipment_changes(&[(EquipmentSlot::HEAD, announcement)]);
}

// The final raid check and assignment commit together. Equipment admission never waits
// while holding raids, and this block performs no equality, events or packet sends.
fn equip_banner(
    mob: &dyn Mob,
    offered: &mut Option<pumpkin_data::item_stack::ItemStack>,
) -> Option<pumpkin_data::item_stack::ItemStack> {
    let raider = mob.as_raider()?;
    let id = raider.get_raider_data().raid_id.load()?;
    let world = mob.get_entity().world.load();
    let mut raids = world
        .raids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let raid = raids.get_mut(id)?;
    if !raid.is_active()
        || raid.is_over()
        || raid.group_to_leader_map.contains_key(&raider.get_wave())
    {
        return None;
    }
    let mut equipment = mob
        .get_mob_entity()
        .living_entity
        .entity_equipment
        .try_lock()
        .ok()?;
    let previous = equipment.put(&EquipmentSlot::HEAD, offered.take()?);
    raid.group_to_leader_map
        .insert(raider.get_wave(), mob.get_entity().entity_uuid);
    Some(previous)
}
