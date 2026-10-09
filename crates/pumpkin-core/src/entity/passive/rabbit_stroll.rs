//! Rabbit's `WaterAvoidingRandomStrollGoal` and its `RandomStrollGoal` lifecycle.
use super::rabbit::STROLL_SPEED_MOD;
use crate::entity::{
    ai::{
        goal::{Controls, Goal, to_goal_ticks, wander_around::DEFAULT_INTERVAL},
        pathfinder::NavigatorGoal,
        util::{default_random_pos, land_random_pos},
    },
    mob::Mob,
};
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use std::sync::atomic::Ordering::Relaxed;

// WaterAvoidingRandomStrollGoal.PROBABILITY and RandomStrollGoal.canUse.
const PROBABILITY: f32 = 0.001;
const MAX_NO_ACTION_TIME: i32 = 100;
// WaterAvoidingRandomStrollGoal.getPosition and RandomStrollGoal.getPosition search sizes.
const LAND_HORIZONTAL_RANGE: i32 = 10;
const WATER_HORIZONTAL_RANGE: i32 = 15;
const VERTICAL_RANGE: i32 = 7;

#[derive(Default)]
pub(super) struct RabbitStrollGoal {
    target: Option<Vector3<f64>>,
}

impl RabbitStrollGoal {
    fn get_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
        let in_water = mob.get_entity().touching_water.load(Relaxed);
        let roll = if in_water {
            0.0
        } else {
            mob.get_random().random::<f32>()
        };
        Self::get_position_with_roll(mob, roll)
    }

    // WaterAvoidingRandomStrollGoal.getPosition; the roll is supplied for deterministic tests.
    fn get_position_with_roll(mob: &dyn Mob, roll: f32) -> Option<Vector3<f64>> {
        if mob.get_entity().touching_water.load(Relaxed) {
            land_random_pos::get_pos(mob, WATER_HORIZONTAL_RANGE, VERTICAL_RANGE)
                .or_else(|| default_random_pos::get_pos(mob, LAND_HORIZONTAL_RANGE, VERTICAL_RANGE))
        } else if roll >= PROBABILITY {
            land_random_pos::get_pos(mob, LAND_HORIZONTAL_RANGE, VERTICAL_RANGE)
        } else {
            default_random_pos::get_pos(mob, LAND_HORIZONTAL_RANGE, VERTICAL_RANGE)
        }
    }
}

impl Goal for RabbitStrollGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob.get_entity().has_passengers()
            || mob
                .get_mob_entity()
                .living_entity
                .no_action_time
                .load(Relaxed)
                >= MAX_NO_ACTION_TIME
            || mob
                .get_random()
                .random_range(0..to_goal_ticks(DEFAULT_INTERVAL))
                != 0
        {
            return false;
        }
        self.target = Self::get_position(mob);
        self.target.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_navigator_idle() && !mob.get_entity().has_passengers()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(target) = self.target {
            mob.get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_progress(NavigatorGoal::new(
                    mob.get_entity().pos.load(),
                    target,
                    STROLL_SPEED_MOD,
                ));
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::ai::util::goal_utils;
    use crate::entity::{Entity, EntityBase, passive::rabbit::RabbitEntity};
    use pumpkin_data::{Block, entity::EntityType};
    use pumpkin_util::math::vector2::Vector2;
    use pumpkin_world::chunk::ChunkData;

    #[tokio::test]
    async fn rabbit_review_wired_stroll_water_selection() {
        let directory = tempfile::tempdir().unwrap();
        let world = crate::entity::living::test_support::armor_test_world(directory.path());
        for cx in -1..=1 {
            for cz in -1..=1 {
                let chunk = ChunkData::empty_sync(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set_block_absolute_y(x, 63, z, Block::STONE.default_state.id);
                        chunk.set_block_absolute_y(x, 64, z, Block::WATER.default_state.id);
                    }
                }
                world
                    .level
                    .loaded_chunks
                    .insert(Vector2::new(cx, cz), chunk);
            }
        }
        let rabbit = RabbitEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 8.5),
            &EntityType::RABBIT,
        ));
        // Remove path malus so water rejection specifically tests LandRandomPos, not malus.
        rabbit
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .set_pathfinding_malus(crate::entity::ai::pathfinder::node::PathType::Water, 0.0);
        rabbit.get_entity().touching_water.store(true, Relaxed);
        let mut installed = {
            let mut selector = rabbit.mob_entity.goals_selector.lock().unwrap();
            // remove_goals returns running goals. Drive the actual registered selector first.
            for _ in 0..4096 {
                selector.tick(rabbit.as_ref());
            }
            selector.remove_goals::<RabbitStrollGoal>()
        };
        assert_eq!(
            installed.len(),
            1,
            "Rabbit.registerGoals installs WaterAvoidingRandomStrollGoal"
        );
        let goal = &mut installed[0];
        goal.stop(rabbit.as_ref());
        // On land, an all-water candidate area never admits a land-search destination.
        rabbit.get_entity().touching_water.store(false, Relaxed);
        assert!(
            (0..16)
                .all(|_| RabbitStrollGoal::get_position_with_roll(rabbit.as_ref(), 0.5).is_none())
        );
        // Exercise the production entry point too, allowing vanilla's rare default-search roll.
        let wet_selections = (0..128)
            .filter(|_| RabbitStrollGoal::get_position(rabbit.as_ref()).is_some())
            .count();
        assert!(wet_selections <= 8, "ordinary strolling must reject water");
        // In water the same installed goal must fall back to RandomStrollGoal.getPosition.
        rabbit.get_entity().touching_water.store(true, Relaxed);
        assert!((0..4096).any(|_| goal.can_start(rabbit.as_ref())));
        goal.start(rabbit.as_ref());
        rabbit
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .tick(&rabbit.mob_entity, rabbit.as_ref());
        let fallback = rabbit
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .get_target_pos()
            .unwrap();
        assert!(goal_utils::is_water(&world, &fallback));
        goal.stop(rabbit.as_ref());

        // Give the land search a dry half of the area. Selected land positions reject water.
        for entry in world.level.loaded_chunks.iter() {
            let chunk = entry.value();
            for x in 0..16 {
                if entry.key().x * 16 + (x as i32) < 8 {
                    for z in 0..16 {
                        chunk.set_block_absolute_y(x, 64, z, Block::AIR.default_state.id);
                    }
                }
            }
        }
        rabbit.get_entity().touching_water.store(false, Relaxed);
        let mut land_count = 0;
        for _ in 0..128 {
            if let Some(position) = RabbitStrollGoal::get_position_with_roll(rabbit.as_ref(), 0.5) {
                assert!(!goal_utils::is_water(
                    &world,
                    &pumpkin_util::math::position::BlockPos::floored_v(position)
                ));
                land_count += 1;
                assert!(position.x < 8.0);
            }
        }
        assert!(land_count > 0);
        crate::server::fixture_lifecycle::finish().await;
    }
}
