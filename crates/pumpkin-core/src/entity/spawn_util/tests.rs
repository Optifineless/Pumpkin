use super::*;
use crate::{
    block::entities::piston::PistonBlockEntity,
    entity::{mob::spawn::SpawnReason, passive::iron_golem::IronGolemEntity},
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{
    BlockDirection,
    biome::Biome,
    block_properties::{Half, MovingPistonLikeProperties, WhiteWoolStairsLikeProperties},
};
use rand::{SeedableRng, rngs::StdRng};

fn spawn_at(fixture: &Fixture, pos: BlockPos, range_y: i32) -> Option<Arc<IronGolemEntity>> {
    try_spawn_mob_with_random(
        &EntityType::IRON_GOLEM,
        SpawnReason::MobSummoned,
        IronGolemEntity::new,
        &fixture.world,
        &pos,
        1,
        0,
        range_y,
        SpawnStrategy::OnTopOfColliderNoLeaves,
        true,
        &mut StdRng::seed_from_u64(1),
    )
}

#[tokio::test]
async fn adjacent_moving_piston_blocks_spawn_on_every_axis() {
    // BlockCollisions' padded cells include PistonMovingBlockEntity at progress zero.
    for (facing, piston_pos) in [
        (BlockDirection::East, BlockPos::new(10, 64, 8)),
        (BlockDirection::West, BlockPos::new(6, 64, 8)),
        (BlockDirection::South, BlockPos::new(8, 64, 10)),
        (BlockDirection::North, BlockPos::new(8, 64, 6)),
        (BlockDirection::Up, BlockPos::new(8, 67, 8)),
        (BlockDirection::Down, BlockPos::new(9, 63, 8)),
    ] {
        let fixture = Fixture::new();
        let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
        let mut props = MovingPistonLikeProperties::default(&Block::MOVING_PISTON);
        props.facing = facing.to_facing();
        terrain.set_block_state(
            piston_pos.0.x,
            piston_pos.0.y,
            piston_pos.0.z,
            BlockState::from_id(props.to_state_id(&Block::MOVING_PISTON)),
        );
        publish(&fixture.world, terrain);
        fixture.world.add_block_entity(Arc::new(PistonBlockEntity {
            position: piston_pos,
            pushed_block_state: Block::STONE.default_state,
            facing,
            current_progress: 0.0.into(),
            last_progress: 0.0.into(),
            extending: true,
            source: false,
        }));
        assert!(spawn_at(&fixture, BlockPos::new(8, 64, 8), 0).is_none());
        fixture.finish().await;
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn upside_down_stairs_union_accepts_both_collider_strategies() {
    let fixture = Fixture::new();
    let pos = BlockPos::new(8, 63, 8);
    let mut stairs = WhiteWoolStairsLikeProperties::default(&Block::OAK_STAIRS);
    stairs.half = Half::Top;
    let state = BlockState::from_id(stairs.to_state_id(&Block::OAK_STAIRS));
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.set_block_state(8, 63, 8, state);
    publish(&fixture.world, terrain);
    // The extracted shape consists of two boxes; Block.isFaceFull accepts their union.
    assert_eq!(state.get_block_collision_shapes().count(), 2);
    for strategy in [
        SpawnStrategy::OnTopOfCollider,
        SpawnStrategy::OnTopOfColliderNoLeaves,
    ] {
        assert!(strategy.can_spawn_on(&fixture.world, &pos, state, Block::AIR.default_state));
    }
    assert!(spawn_at(&fixture, pos.up(), 0).is_some());
    stairs.half = Half::Bottom;
    let bottom = BlockState::from_id(stairs.to_state_id(&Block::OAK_STAIRS));
    assert!(!SpawnStrategy::OnTopOfCollider.can_spawn_on(
        &fixture.world,
        &pos,
        bottom,
        Block::AIR.default_state,
    ));
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn spawn_search_above_build_height_reaches_platform_below() {
    let fixture = Fixture::new();
    let platform_y = fixture.world.get_top_y() - 4;
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.set_block_state(8, platform_y, 8, Block::STONE.default_state);
    publish(&fixture.world, terrain);
    let start = BlockPos::new(8, fixture.world.get_top_y() - 5, 8);
    let golem = spawn_at(&fixture, start, 6).unwrap();
    assert_eq!(
        golem.get_entity().pos.load(),
        Vector3::new(8.5, f64::from(platform_y + 1), 8.5)
    );
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn spawn_collision_padding_still_rejects_unloaded_terrain() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    // The body fits in chunk zero, but its neighboring collision cells are in an unloaded chunk.
    assert!(spawn_at(&fixture, BlockPos::new(14, 64, 8), 0).is_none());
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}
