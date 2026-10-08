//! `PathfinderMob.checkSpawnRules` and species' `getWalkTargetValue` overrides.

use pumpkin_data::{Block, tag::Taggable};
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::Ordering::Relaxed;

use super::Mob;
use crate::world::spawn_view::SpawnView;

pub(super) fn value<M: Mob + ?Sized>(mob: &M, pos: &BlockPos, view: &SpawnView<'_>) -> f32 {
    let entity = mob.get_entity();
    let ty = entity.entity_type;
    let cost = view.pathfinding_cost_from_light(pos);
    let below = view.get_block(&pos.down());
    match ty.resource_name {
        // Axolotl, AbstractNautilus, Enderman and Pillager.getWalkTargetValue.
        "axolotl" | "nautilus" | "zombie_nautilus" | "enderman" | "pillager" | "creaking"
        | "warden" => 0.0,
        "bee" => {
            if view.get_block(pos).is_air() {
                10.0
            } else {
                0.0
            }
        }
        "happy_ghast" => {
            if !view.get_block(pos).is_air() {
                0.0
            } else if view.get_block(&pos.down()).is_air()
                && !view.get_block(&pos.add(0, -2, 0)).is_air()
            {
                10.0
            } else {
                5.0
            }
        }
        "mooshroom" => {
            if below == &Block::MYCELIUM {
                10.0
            } else {
                cost
            }
        }
        // Turtle.getWalkTargetValue; newly spawned turtles have goingHome=false.
        "turtle" => {
            if below.has_tag(&pumpkin_data::tag::Block::MINECRAFT_SAND)
                || view
                    .get_fluid(pos)
                    .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
            {
                10.0
            } else {
                cost
            }
        }
        "strider" => {
            if view
                .get_fluid(pos)
                .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_LAVA)
            {
                10.0
            } else if entity.touching_lava.load(Relaxed) {
                f32::NEG_INFINITY
            } else {
                0.0
            }
        }
        // Hoglin.getWalkTargetValue / HoglinAi.isPosNearNearestRepellent.
        "hoglin" => hoglin_value(mob, pos, below),
        "guardian" | "elder_guardian" => {
            if view
                .get_fluid(pos)
                .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
            {
                10.0 + cost
            } else {
                -cost
            }
        }
        "giant" => cost,
        // Silverfish.getWalkTargetValue / Blocks' InfestedBlock host registrations.
        "silverfish"
            if [
                &Block::STONE,
                &Block::COBBLESTONE,
                &Block::STONE_BRICKS,
                &Block::MOSSY_STONE_BRICKS,
                &Block::CRACKED_STONE_BRICKS,
                &Block::CHISELED_STONE_BRICKS,
                &Block::DEEPSLATE,
            ]
            .contains(&below) =>
        {
            10.0
        }
        _ if super::despawn::inherits_monster(ty) => -cost,
        _ if super::despawn::inherits_animal(ty) => {
            if below == &Block::GRASS_BLOCK {
                10.0
            } else {
                cost
            }
        }
        // WaterAnimal, AgeableWaterCreature, Ghast and other Mob/PathfinderMob bases.
        _ => 0.0,
    }
}

pub fn check<M: Mob + ?Sized>(mob: &M, view: &SpawnView<'_>) -> bool {
    // Mob.checkSpawnRules is true; PathfinderMob checks the subtype's value.
    value(mob, &mob.get_entity().block_pos.load(), view) >= 0.0
}

fn hoglin_value<M: Mob + ?Sized>(mob: &M, pos: &BlockPos, below: &Block) -> f32 {
    let repellent = mob
        .get_mob_entity()
        .brain
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(crate::entity::ai::brain::memory::types::NEAREST_REPELLENT)
        .copied();
    if repellent.is_some_and(|repellent| repellent.squared_distance(pos) < 64) {
        -1.0
    } else if below == &Block::CRIMSON_NYLIUM {
        10.0
    } else {
        0.0
    }
}
