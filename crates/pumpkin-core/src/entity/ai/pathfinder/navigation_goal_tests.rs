use super::super::{
    AmphibiousPathNavigation, FlyingPathNavigation, GroundPathNavigation, Path,
    PathNavigationTrait, WallClimberNavigation, WaterBoundPathNavigation, node::Node,
};
use crate::{
    entity::{Entity, living::test_support::armor_test_world, passive::cow::CowEntity},
    world::World,
};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[expect(
    clippy::unwrap_used,
    reason = "test fixture requires a temporary directory"
)]
fn fixture() -> (tempfile::TempDir, Arc<World>, Arc<CowEntity>) {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let chunk = ChunkData::empty_sync(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_absolute_y(x, 59, z, Block::STONE.default_state.id);
        }
    }
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let cow = CowEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 60.0, 4.5),
        &EntityType::COW,
    ));
    cow.mob_entity
        .living_entity
        .entity
        .on_ground
        .store(true, Relaxed);
    (directory, world, cow)
}

fn replacement_path() -> Path {
    let target = BlockPos::new(6, 60, 4);
    Path::new(vec![Node::new(target)], target, true)
}

#[tokio::test]
async fn direct_path_replacement_survives_navigation_ticks_and_completion() {
    let (_directory, _world, cow) = fixture();
    let mob = &cow.mob_entity;
    let living = &mob.living_entity;
    // All navigator update gates are open; the explicit replacement itself needs no search.
    living.entity.touching_water.store(true, Relaxed);
    let navigators: [(&str, Box<dyn PathNavigationTrait>); 5] = [
        ("ground", Box::new(GroundPathNavigation::new())),
        ("flying", Box::new(FlyingPathNavigation::new())),
        ("water", Box::new(WaterBoundPathNavigation::new(false))),
        ("amphibious", Box::new(AmphibiousPathNavigation::new(false))),
        ("wall climber", Box::new(WallClimberNavigation::new())),
    ];
    for (name, mut navigation) in navigators {
        living.entity.set_pos(Vector3::new(4.5, 60.0, 4.5));
        assert!(navigation.move_to_coords(10.5, 60.0, 4.5, 1.0, living));
        assert!(navigation.move_to_path(Some(replacement_path()), 0.75, living));
        navigation.tick(mob, cow.as_ref());
        assert_eq!(
            navigation.get_path().map(Path::get_target),
            Some(BlockPos::new(6, 60, 4)),
            "{name}: the queued request replaced the explicit path"
        );
        living.entity.set_pos(Vector3::new(6.5, 60.0, 4.5));
        navigation.tick(mob, cow.as_ref());
        assert!(
            navigation.is_done(),
            "{name}: replacement path did not finish"
        );
        navigation.tick(mob, cow.as_ref());
        assert!(navigation.is_done(), "{name}: completed route restarted");
    }
}

#[tokio::test]
async fn absent_or_completed_direct_path_cancels_queued_coordinate_request() {
    let (_directory, _world, cow) = fixture();
    let mob = &cow.mob_entity;
    let living = &mob.living_entity;
    for path in [
        None,
        Some(Path::new(Vec::new(), BlockPos::new(6, 60, 4), true)),
    ] {
        let mut navigation = GroundPathNavigation::new();
        assert!(navigation.move_to_coords(10.5, 60.0, 4.5, 1.0, living));
        assert!(!navigation.move_to_path(path, 1.0, living));
        navigation.tick(mob, cow.as_ref());
        assert!(navigation.is_done());
        assert!(navigation.is_idle());
        assert!(navigation.next_move_target().is_none());
    }
}

#[derive(Clone, Copy, Debug)]
enum Cancellation {
    Stop,
    AbsentPath,
    CompletedPath,
    ReplacementPath,
}

#[tokio::test]
#[expect(
    clippy::unwrap_used,
    reason = "test controls the existing world's clock"
)]
async fn cancelled_deferred_recomputation_cannot_restore_old_route_on_tick() {
    let (_directory, world, cow) = fixture();
    let mob = &cow.mob_entity;
    let living = &mob.living_entity;
    for cancellation in [
        Cancellation::Stop,
        Cancellation::AbsentPath,
        Cancellation::CompletedPath,
        Cancellation::ReplacementPath,
    ] {
        world.level_time.lock().unwrap().world_age = 0;
        let mut navigation = GroundPathNavigation::new();
        assert!(navigation.move_to_coords(10.5, 60.0, 4.5, 1.0, living));
        navigation.tick(mob, cow.as_ref());
        assert!(!navigation.is_done(), "fixture must produce a real route");
        // PathNavigation.recomputePath defers a request within its 20-game-tick debounce.
        navigation.recompute_path(mob);
        match cancellation {
            Cancellation::Stop => navigation.stop(),
            Cancellation::AbsentPath => {
                assert!(!navigation.move_to_path(None, 1.0, living));
            }
            Cancellation::CompletedPath => {
                let path = Path::new(Vec::new(), BlockPos::new(6, 60, 4), true);
                assert!(!navigation.move_to_path(Some(path), 1.0, living));
            }
            Cancellation::ReplacementPath => {
                assert!(navigation.move_to_path(Some(replacement_path()), 1.0, living));
            }
        }
        world.level_time.lock().unwrap().world_age = 21;
        navigation.tick(mob, cow.as_ref());
        if matches!(cancellation, Cancellation::ReplacementPath) {
            assert_eq!(
                navigation.get_path().map(Path::get_target),
                Some(BlockPos::new(6, 60, 4)),
                "{cancellation:?}: deferred recomputation replaced the new path"
            );
        } else {
            assert!(navigation.is_done(), "{cancellation:?}: old route resumed");
            assert!(navigation.is_idle());
            assert!(navigation.next_move_target().is_none());
        }
    }
}

#[tokio::test]
#[expect(clippy::unwrap_used, reason = "the fixture installs this chunk")]
async fn default_stable_destination_requires_solid_render_below() {
    let (_directory, world, cow) = fixture();
    let navigation = WallClimberNavigation::new();
    let ground = GroundPathNavigation::new();
    let chunk = world
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap()
        .clone();
    let target = BlockPos::new(4, 60, 4);
    // PathNavigation.isStableDestination tests the support, even if the target is solid.
    for (support, target_block, expected) in [
        (&Block::STONE, &Block::AIR, true),
        (&Block::GLASS, &Block::AIR, false),
        (&Block::OAK_SLAB, &Block::AIR, false),
        (&Block::AIR, &Block::STONE, false),
    ] {
        chunk.set_block_absolute_y(4, 59, 4, support.default_state.id);
        chunk.set_block_absolute_y(4, 60, 4, target_block.default_state.id);
        assert_eq!(
            navigation.is_stable_destination(&world, &target, cow.as_ref()),
            expected
        );
        assert_eq!(
            ground.is_stable_destination(&world, &target, cow.as_ref()),
            expected
        );
    }
}
