use crate::{
    entity::EntityBase,
    entity::death_test_world::DeathTestWorld,
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, BlockDirection, HorizontalFacingExt, biome::Biome,
    block_properties::LadderLikeProperties, fluid::Fluid, item::Item, item_stack::ItemStack,
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
};
use pumpkin_world::world::BlockFlags;

fn packet(position: BlockPos, face: BlockDirection, sequence: i32) -> SUseItemOn {
    SUseItemOn {
        hand: VarInt(0),
        position,
        face: VarInt(face as i32),
        cursor_pos: Vector3::new(0.5, 0.5, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(sequence),
    }
}

fn surround_with_support(
    chunk: &mut pumpkin_world::generation::proto_chunk::ProtoChunk,
    pos: BlockPos,
) {
    for direction in [
        BlockDirection::North,
        BlockDirection::South,
        BlockDirection::West,
        BlockDirection::East,
    ] {
        let support = pos.offset(direction.to_offset());
        chunk.set_block_state(
            support.0.x,
            support.0.y,
            support.0.z,
            Block::STONE.default_state,
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ladder_placed_in_source_water_preserves_waterlogging() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let water_pos = BlockPos::new(8, 64, 8);
    let dry_pos = BlockPos::new(12, 64, 8);
    let mut chunk = proto(&Biome::PLAINS, &Block::AIR);
    chunk.set_block_state(
        water_pos.0.x,
        water_pos.0.y,
        water_pos.0.z,
        Block::WATER.default_state,
    );
    surround_with_support(&mut chunk, water_pos);
    surround_with_support(&mut chunk, dry_pos);
    let _published_chunk = publish(&world, chunk);

    let player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 6.5));
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::LADDER));

    player
        .client()
        .handle_use_item_on(
            &player.player,
            &packet(water_pos, BlockDirection::Up, 1),
            &fixture.server,
        )
        .unwrap();

    assert_eq!(world.get_block(&water_pos), &Block::LADDER);
    let water_state = world.get_block_state_id(&water_pos);
    let water_properties = LadderLikeProperties::from_state_id(water_state);
    assert!(water_properties.waterlogged);
    let (fluid, fluid_state) = world.get_fluid_and_fluid_state(&water_pos);
    assert!(fluid.matches_type(&Fluid::WATER));
    assert!(fluid_state.is_source);

    // A later neighbor update runs LadderBlock.updateShape for this wet state.
    world.set_block_state(
        &water_pos.up(),
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(world.is_fluid_tick_scheduled(&water_pos, &Fluid::WATER));
    assert_eq!(player.player.inventory().held_item().item_count, 1);

    player
        .player
        .get_entity()
        .set_pos(Vector3::new(12.5, 64.0, 6.5));
    let clicked_support = dry_pos.offset(BlockDirection::North.to_offset());
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &packet(clicked_support, BlockDirection::South, 2),
            &fixture.server,
        )
        .unwrap();

    assert_eq!(world.get_block(&dry_pos), &Block::LADDER);
    let dry_properties = LadderLikeProperties::from_state_id(world.get_block_state_id(&dry_pos));
    assert!(!dry_properties.waterlogged);
    let support_pos = dry_pos.offset(
        dry_properties
            .facing
            .to_block_direction()
            .opposite()
            .to_offset(),
    );
    assert!(
        world
            .get_block_state(&support_pos)
            .is_side_solid(dry_properties.facing.to_block_direction())
    );
    assert!(world.get_fluid(&dry_pos).matches_type(&Fluid::EMPTY));
    assert!(player.player.inventory().held_item().is_empty());

    fixture.server.shutdown().await;
}
