use super::test_support::PlayerFixture;
use super::*;
use crate::entity::Entity;
use crate::{
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::{biome::Biome, block_properties::HorizontalFacing, entity::EntityPose};

fn place(world: &Arc<World>, block: &Block, foot: BlockPos) -> BlockPos {
    let mut properties = BedProperties::default(block);
    properties.facing = HorizontalFacing::East;
    properties.part = BedPart::Foot;
    world.set_block_state(&foot, properties.to_state_id(block), BlockFlags::NOTIFY_ALL);
    foot.offset(properties.facing.to_offset())
}

fn bed_drops(world: &World, block: &Block) -> i32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| {
            let stack = item.get_item_stack().lock().unwrap();
            if stack.item.id == block.item_id {
                i32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

#[tokio::test]
async fn followup2_either_bed_half_drops_once_and_creative_drops_none() {
    for block in [&Block::RED_BED, &Block::STRAW_BED] {
        for part in [BedPart::Foot, BedPart::Head] {
            for destroy_mode in [0, 1, 2] {
                let creative = destroy_mode != 0;
                let fixture = PlayerFixture::new();
                publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
                let foot = BlockPos::new(5, 64, 5);
                let head = place(&fixture.world, block, foot);
                let position = if part == BedPart::Head { head } else { foot };
                if destroy_mode == 2 {
                    super::test_support::creative_break_bedrock(&fixture.world, position).await;
                } else if creative {
                    super::test_support::creative_break_java(&fixture.world, position);
                } else {
                    fixture
                        .world
                        .break_block(&position, None, BlockFlags::NOTIFY_ALL);
                }
                assert_eq!(
                    bed_drops(&fixture.world, block),
                    i32::from(!creative),
                    "{} head={} creative={creative}",
                    block.name,
                    part == BedPart::Head
                );
                assert!(fixture.world.get_block(&foot).is_air());
                assert!(fixture.world.get_block(&head).is_air());
                fixture.finish().await;
            }
        }
    }
}

#[tokio::test]
async fn foot_use_reads_head_occupancy_and_sleep_height() {
    let fixture = PlayerFixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let client = TestPlayer::new(&fixture.world);
    let foot = BlockPos::new(5, 64, 5);
    let head = place(&fixture.world, &Block::RED_BED, foot);
    // A stale foot must not override the authoritative head (AbstractBedBlock.useWithoutItem).
    let mut foot_state = BedProperties::from_state_id(fixture.world.get_block_state_id(&foot));
    foot_state.occupied = true;
    fixture.world.set_block_state(
        &foot,
        foot_state.to_state_id(&Block::RED_BED),
        BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    client.player.get_entity().set_pos(foot.to_centered_f64());
    BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &foot);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(head));
    assert!(client.player.get_entity().pose.load() == EntityPose::Sleeping);
    assert_eq!(client.player.position().y, 64.6875);
    assert!(BedProperties::from_state_id(fixture.world.get_block_state_id(&head)).occupied);
    // LivingEntity.isInWall excludes sleepers even when a forced write replaces their bed cell.
    chunk.set_block_absolute_y(
        head.0.x as usize,
        head.0.y,
        head.0.z as usize,
        Block::STONE.default_state.id,
    );
    assert!(
        client
            .player
            .get_entity()
            .tick_block_collisions(client.player.as_ref())
    );
    let health = client.player.living_entity.health.load();
    let server = fixture.world.server.upgrade().unwrap();
    client
        .player
        .living_entity
        .tick(client.player.as_ref(), &server);
    assert_eq!(client.player.living_entity.health.load(), health);
    fixture.finish().await;
}

#[tokio::test]
async fn bed_partner_updates_occupancy_and_rejects_same_part() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let foot = BlockPos::new(5, 64, 5);
    let head = place(&fixture.world, &Block::RED_BED, foot);
    let mut head_state = BedProperties::from_state_id(fixture.world.get_block_state_id(&head));
    head_state.occupied = true;
    fixture.world.set_block_state(
        &head,
        head_state.to_state_id(&Block::RED_BED),
        BlockFlags::NOTIFY_ALL,
    );
    assert!(BedProperties::from_state_id(fixture.world.get_block_state_id(&foot)).occupied);
    head_state.part = BedPart::Foot;
    fixture.world.set_block_state(
        &head,
        head_state.to_state_id(&Block::RED_BED),
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
    );
    assert!(fixture.world.get_block(&foot).is_air());
    fixture.finish().await;
}

#[test]
fn bed_shapes_match_vanilla_height_and_connected_direction() {
    // BedBlock.getShape rotates the same base and legs through getConnectedDirection.
    for block in [&Block::WHITE_BED, &Block::RED_BED] {
        let mut props = BedProperties::default(block);
        props.facing = HorizontalFacing::East;
        props.part = BedPart::Foot;
        let foot = props.to_state_id(block).to_state();
        props.facing = HorizontalFacing::West;
        props.part = BedPart::Head;
        let head = props.to_state_id(block).to_state();
        assert_eq!(foot.collision_shapes, head.collision_shapes);
        assert!(!foot.is_full_cube());
        assert_eq!(
            foot.get_block_collision_shapes()
                .map(|shape| shape.max.y)
                .fold(0.0, f64::max),
            9.0 / 16.0
        );
    }
    let mut straw = BedProperties::default(&Block::STRAW_BED);
    straw.part = BedPart::Foot;
    assert_eq!(
        straw
            .to_state_id(&Block::STRAW_BED)
            .to_state()
            .get_block_collision_shapes()
            .map(|shape| shape.max.y)
            .fold(0.0, f64::max),
        0.25
    );
}

#[tokio::test]
async fn orphan_foot_cannot_sleep_inside_replacement_block() {
    let fixture = PlayerFixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let foot = BlockPos::new(5, 64, 5);
    let head = place(&fixture.world, &Block::RED_BED, foot);
    // Stale state from disk or a forced write must not teleport a sleeper into a solid head cell.
    chunk.set_block_absolute_y(
        head.0.x as usize,
        head.0.y,
        head.0.z as usize,
        Block::STONE.default_state.id,
    );
    let client = TestPlayer::new(&fixture.world);
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    client.player.get_entity().set_pos(foot.to_centered_f64());
    assert!(matches!(
        BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &foot),
        BlockActionResult::Consume
    ));
    assert!(client.player.sleeping_bed_pos.load().is_none());
    fixture.finish().await;
}

#[tokio::test]
async fn isolated_bed_head_still_drops_once() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let mut props = BedProperties::default(&Block::RED_BED);
    props.part = BedPart::Head;
    let pos = BlockPos::new(5, 64, 5);
    chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::RED_BED));
    fixture
        .world
        .break_block(&pos, None, BlockFlags::NOTIFY_ALL);
    assert_eq!(bed_drops(&fixture.world, &Block::RED_BED), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn occupied_foot_wakes_villager_before_obstruction_check() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let foot = BlockPos::new(5, 64, 5);
    let head = place(&fixture.world, &Block::RED_BED, foot);
    let villager = crate::entity::passive::villager::VillagerEntity::new(Entity::new(
        fixture.world.clone(),
        head.to_centered_f64(),
        &EntityType::VILLAGER,
    ));
    *villager.home_pos.lock().unwrap() = Some(head);
    villager
        .get_entity()
        .set_pose(pumpkin_data::entity::EntityPose::Sleeping);
    fixture.world.add_entity_silent(villager.clone());
    BedBlock::set_occupied(
        true,
        &fixture.world,
        &Block::RED_BED,
        &head,
        fixture.world.get_block_state_id(&head),
    );
    fixture.world.set_block_state(
        &head.up(),
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    let client = TestPlayer::new(&fixture.world);
    BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &foot);
    assert!(villager.get_entity().pose.load() == pumpkin_data::entity::EntityPose::Standing);
    assert!(client.player.sleeping_bed_pos.load().is_none());
    assert!(!BedProperties::from_state_id(fixture.world.get_block_state_id(&head)).occupied);
    fixture.finish().await;
}
