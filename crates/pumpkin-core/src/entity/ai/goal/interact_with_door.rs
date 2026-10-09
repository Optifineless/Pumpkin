use pumpkin_data::{
    Block,
    block_properties::OakDoorLikeProperties,
    tag::{self, Taggable},
};
use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use super::{Controls, Goal};
use crate::{
    block::blocks::doors::DoorBlock,
    entity::{
        ai::brain::memory::{GlobalPos, types},
        mob::Mob,
    },
};

// InteractWithDoor's distance and retry constants.
const COOLDOWN_BEFORE_RERUNNING_IN_SAME_NODE: i32 = 20;
const SKIP_CLOSING_DOOR_IF_FURTHER_AWAY_THAN: f64 = 3.0;
const MAX_DISTANCE_TO_HOLD_DOOR_OPEN_FOR_OTHER_MOBS: f64 = 2.0;

#[derive(Default)]
pub struct InteractWithDoorGoal {
    last_node: Option<BlockPos>,
    cooldown: i32,
}

fn path_nodes(mob: &dyn Mob) -> (Option<BlockPos>, Option<BlockPos>) {
    let navigator = mob
        .get_mob_entity()
        .navigator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(path) = navigator.get_path().filter(|path| !path.is_done()) else {
        return (None, None);
    };
    (
        path.get_previous_node().map(|node| node.pos),
        path.get_next_node_pos(),
    )
}

fn interactable(block: &Block) -> bool {
    block.has_tag(&tag::Block::MINECRAFT_MOB_INTERACTABLE_DOORS)
}

/// Closes remembered doors after passage, leaving doors used by another nearby villager open.
pub(crate) fn close_doors(mob: &dyn Mob, from: Option<BlockPos>, to: Option<BlockPos>) {
    // InteractWithDoor.closeDoorsThatIHaveOpenedOrPassedThrough / areOtherMobsComingThroughDoor.
    let entity = mob.get_entity();
    let world = entity.world.load_full();
    let mut doors = {
        let mut brain = mob
            .get_mob_entity()
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let doors = brain
            .get_mut(types::DOORS_TO_CLOSE)
            .map(std::mem::take)
            .unwrap_or_default();
        brain.erase(types::DOORS_TO_CLOSE.id());
        doors
    };
    doors.retain(|door| {
        if Some(door.pos) == from || Some(door.pos) == to {
            return true;
        }
        if door.dimension.id != world.dimension.id
            || door
                .pos
                .to_centered_f64()
                .squared_distance_to_vec(&entity.pos.load())
                >= SKIP_CLOSING_DOOR_IF_FURTHER_AWAY_THAN.powi(2)
        {
            return false;
        }
        let (block, state) = world.get_block_and_state_id(&door.pos);
        if !interactable(block) || !OakDoorLikeProperties::from_state_id(state).open {
            return false;
        }
        let bounds = pumpkin_util::math::boundingbox::BoundingBox::full_block()
            .at_pos(door.pos)
            .expand(
                MAX_DISTANCE_TO_HOLD_DOOR_OPEN_FOR_OTHER_MOBS,
                MAX_DISTANCE_TO_HOLD_DOOR_OPEN_FOR_OTHER_MOBS,
                MAX_DISTANCE_TO_HOLD_DOOR_OPEN_FOR_OTHER_MOBS,
            );
        let other_coming = world.get_entities_at_box(&bounds).iter().any(|other| {
            let other_entity = other.get_entity();
            if other_entity.entity_uuid == entity.entity_uuid
                || other_entity.entity_type != entity.entity_type
                || door
                    .pos
                    .to_centered_f64()
                    .squared_distance_to_vec(&other_entity.pos.load())
                    >= MAX_DISTANCE_TO_HOLD_DOOR_OPEN_FOR_OTHER_MOBS.powi(2)
            {
                return false;
            }
            other.get_mob().is_some_and(|other| {
                let (previous, next) = path_nodes(other);
                previous.is_some() && (previous == Some(door.pos) || next == Some(door.pos))
            })
        });
        if !other_coming {
            DoorBlock::set_open_with_source(&world, &door.pos, false, Some(entity));
        }
        false
    });
    if !doors.is_empty() {
        mob.get_mob_entity()
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(types::DOORS_TO_CLOSE, doors);
    }
}

impl Goal for InteractWithDoorGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        path_nodes(mob).1.is_some()
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }
    fn tick(&mut self, mob: &dyn Mob) {
        // InteractWithDoor.create: open the previous and next path nodes before reaching a slab.
        let (from, to) = path_nodes(mob);
        if to == self.last_node {
            self.cooldown = COOLDOWN_BEFORE_RERUNNING_IN_SAME_NODE;
        } else {
            self.cooldown -= 1;
            if self.cooldown > 0 {
                return;
            }
        }
        self.last_node = to;
        let world = mob.get_entity().world.load_full();
        let mut remember = FxHashSet::default();
        for pos in [from, to].into_iter().flatten() {
            let (block, state) = world.get_block_and_state_id(&pos);
            if !interactable(block) {
                continue;
            }
            let open = OakDoorLikeProperties::from_state_id(state).open;
            if !open {
                DoorBlock::set_open_with_source(&world, &pos, true, Some(mob.get_entity()));
            }
            if (Some(pos) == from || !open)
                && let Some(dimension) =
                    pumpkin_data::dimension::Dimension::from_name(world.dimension.minecraft_name)
            {
                remember.insert(GlobalPos::new(dimension, pos));
            }
        }
        if !remember.is_empty() {
            let mut brain = mob
                .get_mob_entity()
                .brain
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(doors) = brain.get_mut(types::DOORS_TO_CLOSE) {
                doors.extend(remember);
            } else {
                brain.set(types::DOORS_TO_CLOSE, remember);
            }
        }
        close_doors(mob, from, to);
    }
    fn stop(&mut self, mob: &dyn Mob) {
        close_doors(mob, None, None);
    }
    fn should_run_every_tick(&self) -> bool {
        true
    }
    fn controls(&self) -> Controls {
        Controls::empty()
    }
}
