use super::*;
use crate::entity::Entity;
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::chunk::ChunkData;

#[tokio::test]
async fn avoidance_followup_wall_climber_rejects_glass_destination() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let chunk = ChunkData::empty_sync(0, 0);
    let candidate = BlockPos::new(4, 64, 4);
    chunk.set_block_absolute_y(4, 63, 4, Block::GLASS.default_state.id);
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), chunk.clone());
    let entity = Entity::new(
        world.clone(),
        candidate.to_centered_f64(),
        &EntityType::SPIDER,
    );
    let navigation = WallClimberNavigation::new();
    assert!(!navigation.is_stable_destination(&world, &candidate, &entity));
    chunk.set_block_absolute_y(4, 63, 4, Block::STONE.default_state.id);
    assert!(navigation.is_stable_destination(&world, &candidate, &entity));
}

fn assert_replacement(mut navigation: impl PathNavigationTrait, mob: &MobEntity) {
    let destination = BlockPos::new(4, 64, 4);
    let path = Path::new(
        vec![Node::new(destination), Node::new(destination.east())],
        destination.east(),
        true,
    );
    let stale = NavigatorGoal::new(
        Vector3::new(0.5, 64.0, 0.5),
        Vector3::new(9.5, 64.0, 9.5),
        2.0,
    );
    navigation.set_progress(stale);
    assert!(navigation.move_to_path(Some(path.clone()), 1.0, &mob.living_entity));
    navigation.tick(mob, &mob.living_entity);
    assert_eq!(navigation.get_path(), Some(&path));
    navigation.set_progress(stale);
    assert!(!navigation.move_to_path(None, 1.0, &mob.living_entity));
    navigation.tick(mob, &mob.living_entity);
    assert!(navigation.get_path().is_none());
    navigation.set_progress(stale);
    let empty = Path::new(Vec::new(), destination, false);
    assert!(!navigation.move_to_path(Some(empty.clone()), 1.0, &mob.living_entity));
    navigation.tick(mob, &mob.living_entity);
    assert_eq!(navigation.get_path(), Some(&empty));
}

#[tokio::test]
async fn avoidance_followup_installed_path_supersedes_queued_move_for_all_navigators() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let mob = MobEntity::new(Entity::new(
        world,
        Vector3::new(0.5, 64.0, 0.5),
        &EntityType::SPIDER,
    ));
    mob.living_entity
        .entity
        .on_ground
        .store(true, Ordering::Relaxed);
    assert_replacement(GroundPathNavigation::new(), &mob);
    assert_replacement(FlyingPathNavigation::new(), &mob);
    assert_replacement(WaterBoundPathNavigation::new(false), &mob);
    assert_replacement(AmphibiousPathNavigation::new(false), &mob);
    assert_replacement(WallClimberNavigation::new(), &mob);
}
