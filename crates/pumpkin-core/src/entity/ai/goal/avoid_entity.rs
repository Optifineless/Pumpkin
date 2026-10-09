use std::sync::Arc;

use super::{Controls, Goal};
use crate::entity::ai::util::default_random_pos;
use crate::entity::ai::{pathfinder::path::Path, target_predicate::TargetPredicate};
use crate::entity::{EntityBase, mob::Mob};
use pumpkin_data::entity::EntityType;

const FAST_DISTANCE_SQ: f64 = 49.0;
const HORIZONTAL_RANGE: i32 = 16;
const VERTICAL_RANGE: i32 = 7;
// AvoidEntityGoal.canUse inflates the mob's bounding box by three blocks vertically.
const THREAT_VERTICAL_RANGE: f64 = 3.0;

/// Filters candidate threats for the avoiding mob before nearest selection.
pub type AvoidPredicate = dyn Fn(&dyn Mob, &dyn EntityBase) -> bool + Send + Sync;

pub struct AvoidEntityGoal {
    goal_control: Controls,
    flee_type: &'static EntityType,
    flee_distance: f64,
    slow_speed: f64,
    fast_speed: f64,
    target: Option<Arc<dyn EntityBase>>,
    path: Option<Path>,
    avoid_predicate: Option<Box<AvoidPredicate>>,
}

impl AvoidEntityGoal {
    #[must_use]
    pub fn new(
        flee_type: &'static EntityType,
        flee_distance: f64,
        slow_speed: f64,
        fast_speed: f64,
        avoid_predicate: Option<Box<AvoidPredicate>>,
    ) -> Self {
        Self {
            goal_control: Controls::MOVE,
            flee_type,
            flee_distance,
            slow_speed,
            fast_speed,
            target: None,
            path: None,
            avoid_predicate,
        }
    }

    /// Selects the nearest visible, attackable threat inside `AvoidEntityGoal`'s search box.
    /// The selector supplies the avoided class or species predicate before nearest selection.
    pub fn find_threat(
        mob: &dyn Mob,
        distance: f64,
        selector: impl Fn(&dyn EntityBase) -> bool,
    ) -> Option<Arc<dyn EntityBase>> {
        // AvoidEntityGoal.canUse delegates threat filtering to TargetingConditions.test.
        let entity = mob.get_entity();
        let pos = entity.pos.load();
        let world = entity.world.load();
        let search_box =
            entity
                .bounding_box
                .load()
                .expand(distance, THREAT_VERTICAL_RANGE, distance);
        let targeting = TargetPredicate::create_attackable().set_base_max_distance(distance);
        let mut nearest = None;
        let mut nearest_distance = f64::INFINITY;
        world.for_each_in_box(&search_box, |target| {
            if selector(target.as_ref())
                // EntitySelector.NO_CREATIVE_OR_SPECTATOR needs the owning Player, not its base Entity.
                && !target.get_player().is_some_and(|player| player.is_creative() || player.is_spectator())
                && targeting.test(&world, Some(mob), target.as_ref())
            {
                let distance = target.get_entity().pos.load().squared_distance_to_vec(&pos);
                if distance < nearest_distance {
                    nearest_distance = distance;
                    nearest = Some(target.clone());
                }
            }
        });
        nearest
    }

    /// Returns the threat admitted together with the escape path.
    pub(super) fn threat(&self) -> Option<&dyn EntityBase> {
        self.target.as_deref()
    }

    fn matches_avoided_class(&self, entity_type: &EntityType) -> bool {
        // AvoidEntityGoal.canUse queries a Java class, including its subclasses.
        entity_type == self.flee_type
            || self.flee_type == &EntityType::LLAMA && entity_type == &EntityType::TRADER_LLAMA
            || self.flee_type == &EntityType::ZOMBIE
                && [
                    &EntityType::HUSK,
                    &EntityType::DROWNED,
                    &EntityType::ZOMBIE_VILLAGER,
                    &EntityType::ZOMBIFIED_PIGLIN,
                ]
                .contains(&entity_type)
    }
}

impl Goal for AvoidEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(target) = Self::find_threat(mob, self.flee_distance, |target| {
            let entity_type = target.get_entity().entity_type;
            self.matches_avoided_class(entity_type)
                && self
                    .avoid_predicate
                    .as_ref()
                    .is_none_or(|predicate| predicate(mob, target))
        }) else {
            return false;
        };

        let threat_pos = target.get_entity().pos.load();
        let Some(flee_pos) =
            default_random_pos::get_pos_away(mob, HORIZONTAL_RANGE, VERTICAL_RANGE, threat_pos)
        else {
            return false;
        };

        // Give up when the escape route does not gain any distance.
        let mob_pos = mob.get_entity().pos.load();
        if threat_pos.squared_distance_to_vec(&flee_pos)
            < threat_pos.squared_distance_to_vec(&mob_pos)
        {
            return false;
        }

        // AvoidEntityGoal.canUse must admit a path with reach range zero before start.
        let path = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .create_path(mob.get_mob_entity(), flee_pos, 0);
        if path.is_none() {
            return false;
        }
        self.target = Some(target);
        self.path = path;
        true
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_navigator_idle()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(path) = self.path.clone() {
            let mut navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            navigator.move_to_path(
                Some(path),
                self.slow_speed,
                &mob.get_mob_entity().living_entity,
            );
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(target) = &self.target {
            let mob_pos = mob.get_mob_entity().living_entity.entity.pos.load();
            let threat_pos = target.get_entity().pos.load();
            let dist_sq = mob_pos.squared_distance_to_vec(&threat_pos);
            let speed = if dist_sq < FAST_DISTANCE_SQ {
                self.fast_speed
            } else {
                self.slow_speed
            };
            let mut navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            navigator.set_speed(speed);
        }
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.target = None;
        self.path = None;
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entity::{Entity, passive::rabbit::RabbitEntity},
        net::java::combat_test_support::TestPlayer,
        server::combat_test_support,
    };
    use pumpkin_data::Block;
    use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
    use pumpkin_world::chunk::ChunkData;
    use std::sync::atomic::Ordering::Relaxed;

    #[tokio::test]
    async fn rabbit_review_visible_threat_and_escape_path() {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        for cx in -1..=1 {
            for cz in -1..=1 {
                let chunk = ChunkData::empty_sync(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set_block_absolute_y(x, 63, z, Block::STONE.default_state.id);
                    }
                }
                world
                    .level
                    .loaded_chunks
                    .insert(Vector2::new(cx, cz), chunk);
            }
        }
        let witness = TestPlayer::new(&world);
        witness
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        let rabbit = RabbitEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 8.5),
            &EntityType::RABBIT,
        ));
        rabbit.get_entity().on_ground.store(true, Relaxed);
        let chunk = world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap()
            .clone();
        for y in 64..=68 {
            for z in 0..16 {
                chunk.set_block_absolute_y(9, y, z, Block::STONE.default_state.id);
            }
        }
        let mut goal = AvoidEntityGoal::new(&EntityType::PLAYER, 8.0, 2.2, 2.2, None);
        assert!(AvoidEntityGoal::find_threat(rabbit.as_ref(), 8.0, |_| true).is_none());
        assert!(
            !goal.can_start(rabbit.as_ref()),
            "a solid wall hides the player"
        );
        for y in 64..=68 {
            for z in 0..16 {
                chunk.set_block_absolute_y(9, y, z, Block::AIR.default_state.id);
            }
        }
        rabbit.mob_entity.sensing.lock().unwrap().tick();
        assert!(AvoidEntityGoal::find_threat(rabbit.as_ref(), 8.0, |_| true).is_some());
        witness
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 68.0, 8.5));
        rabbit.mob_entity.sensing.lock().unwrap().tick();
        assert!(
            !goal.can_start(rabbit.as_ref()),
            "the player is above the search box"
        );
        witness
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        rabbit.mob_entity.sensing.lock().unwrap().tick();
        rabbit.get_entity().on_ground.store(false, Relaxed);
        for _ in 0..256 {
            assert!(
                !goal.can_start(rabbit.as_ref()),
                "ground navigation cannot create an airborne path"
            );
        }
        rabbit.get_entity().on_ground.store(true, Relaxed);
        // Vanilla permits partial escape paths; choose a reachable sample to inspect reach range.
        assert!(
            (0..256).any(|_| goal.can_start(rabbit.as_ref())
                && goal.path.as_ref().is_some_and(Path::can_reach))
        );
        let path = goal.path.as_ref().unwrap().clone();
        assert!(path.can_reach());
        assert_eq!(path.get_end_node().unwrap().pos, path.get_target());
        goal.start(rabbit.as_ref());
        assert_eq!(
            rabbit.mob_entity.navigator.lock().unwrap().get_path(),
            Some(&path)
        );
    }
}

#[cfg(test)]
#[path = "avoid_entity_class_tests.rs"]
mod class_tests;
