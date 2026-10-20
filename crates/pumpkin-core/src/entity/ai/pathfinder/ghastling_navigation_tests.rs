use super::*;
use crate::entity::{Entity, living::test_support::armor_test_world, passive::cow::CowEntity};
use pumpkin_data::{
    Block,
    block_properties::{HorizontalFacing, OakDoorLikeProperties},
    entity::EntityType,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::chunk::ChunkData;
use std::sync::{Arc, atomic::Ordering::Relaxed};

fn fixture() -> (tempfile::TempDir, Arc<World>, Arc<CowEntity>) {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    for x in 0..=2 {
        for z in 0..=1 {
            world
                .level
                .loaded_chunks
                .insert(Vector2::new(x, z), ChunkData::empty_sync(x, z));
        }
    }
    let mob = CowEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.6, 70.0, 8.5),
        &EntityType::COW,
    ));
    (directory, world, mob)
}

fn set(world: &World, pos: BlockPos, block: &'static Block) {
    let chunk = world
        .level
        .loaded_chunks
        .get(&Vector2::new(pos.0.x >> 4, pos.0.z >> 4))
        .unwrap();
    let state = if block == &Block::OAK_DOOR {
        let mut door = OakDoorLikeProperties::default(block);
        door.facing = HorizontalFacing::West;
        door.to_state_id(block)
    } else {
        block.default_state.id
    };
    chunk.set_block_absolute_y(
        (pos.0.x & 15) as usize,
        pos.0.y,
        (pos.0.z & 15) as usize,
        state,
    );
}

fn corner() -> super::super::path::Path {
    use super::super::{node::Node, path::Path};
    Path::new(
        vec![
            Node::new(BlockPos::new(6, 70, 8)),
            Node::new(BlockPos::new(7, 70, 8)),
        ],
        BlockPos::new(7, 70, 8),
        true,
    )
}

#[tokio::test]
async fn feat_happy_ghast_s2_navigation_corner_ignores_only_fluids() {
    let (_directory, world, mob) = fixture();
    let living = &mob.mob_entity.living_entity;
    for (block, expected) in [
        (&Block::WATER, 1),
        (&Block::LAVA, 1),
        (&Block::STONE, 0),
        (&Block::OAK_DOOR, 0),
    ] {
        set(&world, BlockPos::new(5, 70, 8), block);
        for (name, mut navigation, next) in [
            ("generic flying", Navigator::flying(), 0),
            ("ghastling", Navigator::ghastling(), expected),
        ] {
            assert!(navigation.move_to_path(Some(corner()), 1.0, living));
            navigation.tick(&mob.mob_entity, mob.as_ref());
            assert_eq!(
                navigation.get_path().unwrap().get_next_node_index(),
                next,
                "{name} corner through {}",
                block.name
            );
        }
    }
    world.level.loaded_chunks.remove(&Vector2::new(0, 0));
    let mut navigation = Navigator::ghastling();
    assert!(navigation.move_to_path(Some(corner()), 1.0, living));
    navigation.tick(&mob.mob_entity, mob.as_ref());
    assert_eq!(navigation.get_path().unwrap().get_next_node_index(), 0);
    assert!(!world.is_loaded(&BlockPos::new(5, 70, 8)));
}

#[tokio::test]
async fn feat_happy_ghast_s2_navigation_search_detours_beyond_sixteen() {
    let (_directory, world, mob) = fixture();
    for y in 65..=76 {
        for z in 4..=12 {
            set(&world, BlockPos::new(16, y, z), &Block::STONE);
        }
    }
    let target = Vector3::new(28.5, 70.0, 8.5);
    let mut control = Navigator::flying();
    assert!(
        !control
            .create_path(&mob.mob_entity, target, 0)
            .is_some_and(|path| path.can_reach()),
        "stock sixteen-block search unexpectedly reached the distant target"
    );
    let mut navigation = Navigator::ghastling();
    let path = navigation.create_path(&mob.mob_entity, target, 0).unwrap();
    assert!(path.can_reach(), "48-block search did not reach target");
    assert_eq!(path.get_target(), BlockPos::new(28, 70, 8));
    assert!(
        (0..path.get_node_count()).any(|i| {
            path.get_node_pos(i)
                .is_some_and(|pos| pos.0.y < 65 || pos.0.y > 76 || pos.0.z < 4 || pos.0.z > 12)
        }),
        "route did not detour around the solid wall"
    );
}

#[tokio::test]
async fn feat_happy_ghast_s2_navigation_float_starts_above_water() {
    let (_directory, world, mob) = fixture();
    for y in 70..=73 {
        set(&world, BlockPos::new(4, y, 8), &Block::WATER);
    }
    mob.get_entity().touching_water.store(true, Relaxed);
    let target = Vector3::new(8.5, 75.0, 8.5);
    let mut control = Navigator::flying();
    let path = control.create_path(&mob.mob_entity, target, 0).unwrap();
    assert_eq!(path.get_node_pos(0).unwrap().0.y, 70);
    let mut navigation = Navigator::ghastling();
    let path = navigation.create_path(&mob.mob_entity, target, 0).unwrap();
    assert_eq!(path.get_node_pos(0).unwrap().0.y, 74);
    assert!(path.can_reach());
}
