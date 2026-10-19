use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{
        ChestLikeProperties, ChestType, DoubleBlockHalf, Half, HorizontalFacing,
        OakDoorLikeProperties, StairsShape, WhiteWoolStairsLikeProperties,
    },
    item::Item,
    item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn, ser::NetworkReadExt};
use pumpkin_util::{Hand, math::position::BlockPos, math::vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const CLICKED: BlockPos = BlockPos::new(0, 64, 1);

fn fixture() -> (tempfile::TempDir, Arc<Server>, Arc<World>, TestPlayer) {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 0.5));
    player.player.get_entity().set_sneaking(true);
    player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::IRON_AXE));
    (directory, server, world, player)
}

fn use_on(player: &TestPlayer, server: &Arc<Server>) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: CLICKED,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            server,
        )
        .unwrap();
}

fn replace_before_write(
    world: &Arc<World>,
    position: BlockPos,
    replacement: &'static Block,
) -> Arc<AtomicBool> {
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let weak = Arc::downgrade(world);
    *world.persistence_test_hooks.transform_write.lock().unwrap() =
        Some(Arc::new(move |writing| {
            if writing == position && !observed.swap(true, Ordering::Relaxed) {
                weak.upgrade().unwrap().set_block_state(
                    &position,
                    replacement.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
            }
        }));
    called
}

fn damage(player: &TestPlayer) -> i32 {
    player
        .player
        .inventory()
        .get_stack_in_hand(Hand::Right)
        .get_damage()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_review_java_neighbor_shape_conflict_retries_from_live_stair() {
    let (_directory, server, world, player) = fixture();
    let mut observer = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        player.player.clone(),
        observer.player.clone(),
    ]));
    let mut clicked = WhiteWoolStairsLikeProperties::default(&Block::WAXED_CUT_COPPER_STAIRS);
    clicked.facing = HorizontalFacing::North;
    clicked.half = Half::Bottom;
    clicked.shape = StairsShape::Straight;
    world.set_block_state(
        &CLICKED,
        clicked.to_state_id(&Block::WAXED_CUT_COPPER_STAIRS),
        BlockFlags::FORCE_STATE,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let weak = Arc::downgrade(&world);
    *world.persistence_test_hooks.transform_write.lock().unwrap() =
        Some(Arc::new(move |position| {
            if position == CLICKED && observed.fetch_add(1, Ordering::Relaxed) == 0 {
                let world = weak.upgrade().unwrap();
                let mut neighbor = WhiteWoolStairsLikeProperties::default(&Block::STONE_STAIRS);
                neighbor.facing = HorizontalFacing::East;
                neighbor.half = Half::Bottom;
                world.set_block_state(
                    &CLICKED.offset(Vector3::new(0, 0, -1)),
                    neighbor.to_state_id(&Block::STONE_STAIRS),
                    BlockFlags::NOTIFY_ALL,
                );
                assert_eq!(world.get_block(&CLICKED), &Block::WAXED_CUT_COPPER_STAIRS);
                assert_eq!(
                    WhiteWoolStairsLikeProperties::from_state_id(
                        world.get_block_state_id(&CLICKED)
                    )
                    .shape,
                    StairsShape::OuterRight,
                    "ordinary neighbor notifications must change the observed clicked state"
                );
            }
        }));
    observer.take_packets();

    use_on(&player, &server);

    assert_eq!(world.get_block(&CLICKED), &Block::CUT_COPPER_STAIRS);
    clicked.shape = StairsShape::OuterRight;
    assert_eq!(
        world.get_block_state_id(&CLICKED),
        clicked.to_state_id(&Block::CUT_COPPER_STAIRS),
        "retry must preserve the live stair properties"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(damage(&player), 1);
    assert_eq!(
        player
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::IRON_AXE.id)),
        1
    );
    let packets: Vec<_> = observer
        .take_packets()
        .into_iter()
        .map(|packet| packet.as_ref().get_var_int().unwrap().0)
        .collect();
    for kind in [
        pumpkin_data::packet::clientbound::play::SOUND,
        pumpkin_data::packet::clientbound::play::LEVEL_EVENT,
    ] {
        assert_eq!(
            packets
                .iter()
                .filter(|id| **id == kind.to_id(pumpkin_data::packet::CURRENT_MC_VERSION))
                .count(),
            1,
            "the successful transformation must publish each effect once"
        );
    }
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_review_java_repeated_property_conflicts_are_bounded() {
    let (_directory, server, world, player) = fixture();
    world.set_block_state(
        &CLICKED,
        Block::WAXED_CUT_COPPER_STAIRS.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let weak = Arc::downgrade(&world);
    *world.persistence_test_hooks.transform_write.lock().unwrap() =
        Some(Arc::new(move |position| {
            if position == CLICKED {
                assert!(
                    observed.fetch_add(1, Ordering::Relaxed) < 8,
                    "unbounded axe retry"
                );
                let world = weak.upgrade().unwrap();
                let mut live = WhiteWoolStairsLikeProperties::from_state_id(
                    world.get_block_state_id(&CLICKED),
                );
                live.waterlogged = !live.waterlogged;
                world.set_block_state(
                    &CLICKED,
                    live.to_state_id(&Block::WAXED_CUT_COPPER_STAIRS),
                    BlockFlags::FORCE_STATE,
                );
            }
        }));

    use_on(&player, &server);

    assert_eq!(calls.load(Ordering::Relaxed), 3);
    assert_eq!(world.get_block(&CLICKED), &Block::WAXED_CUT_COPPER_STAIRS);
    assert_eq!(damage(&player), 0);
    assert_eq!(
        player
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::IRON_AXE.id)),
        0
    );
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_review_java_partner_replacement_survives_transform() {
    for replacement in [&Block::AIR, &Block::DIAMOND_BLOCK] {
        let (_directory, server, world, player) = fixture();
        let partner = CLICKED.offset(Vector3::new(1, 0, 0));
        for (position, half) in [(CLICKED, ChestType::Left), (partner, ChestType::Right)] {
            let mut properties = ChestLikeProperties::default(&Block::WAXED_COPPER_CHEST);
            properties.facing = HorizontalFacing::North;
            properties.r#type = half;
            world.set_block_state(
                &position,
                properties.to_state_id(&Block::WAXED_COPPER_CHEST),
                BlockFlags::FORCE_STATE,
            );
        }
        let called = replace_before_write(&world, partner, replacement);

        use_on(&player, &server);

        assert!(called.load(Ordering::Relaxed));
        assert_eq!(world.get_block(&partner), replacement);
        assert_eq!(world.get_block(&CLICKED), &Block::COPPER_CHEST);
        assert_eq!(damage(&player), 1);
        world.level.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_review_java_door_companion_replacement_survives_transform() {
    let (_directory, server, world, player) = fixture();
    for (position, half) in [
        (CLICKED, DoubleBlockHalf::Lower),
        (CLICKED.up(), DoubleBlockHalf::Upper),
    ] {
        let mut properties = OakDoorLikeProperties::default(&Block::WAXED_COPPER_DOOR);
        properties.half = half;
        world.set_block_state(
            &position,
            properties.to_state_id(&Block::WAXED_COPPER_DOOR),
            BlockFlags::FORCE_STATE,
        );
    }
    let called = replace_before_write(&world, CLICKED.up(), &Block::DIAMOND_BLOCK);

    use_on(&player, &server);

    assert!(called.load(Ordering::Relaxed));
    assert_eq!(world.get_block(&CLICKED.up()), &Block::DIAMOND_BLOCK);
    // Replacing a real door half also destroys its old companion before our CAS.
    assert_eq!(world.get_block(&CLICKED), &Block::AIR);
    assert_eq!(damage(&player), 0);
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_review_java_rejected_clicked_transform_keeps_item_and_skips_effects() {
    let (_directory, server, world, player) = fixture();
    let mut observer = TestPlayer::new(&world);
    observer
        .player
        .get_entity()
        .set_pos(Vector3::new(1.5, 64.0, 0.5));
    world.players.store(Arc::new(vec![
        player.player.clone(),
        observer.player.clone(),
    ]));
    world.set_block_state(
        &CLICKED,
        Block::WAXED_COPPER_BLOCK.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let called = replace_before_write(&world, CLICKED, &Block::DIAMOND_BLOCK);
    observer.take_packets();

    use_on(&player, &server);

    assert!(called.load(Ordering::Relaxed));
    assert_eq!(world.get_block(&CLICKED), &Block::DIAMOND_BLOCK);
    assert_eq!(damage(&player), 0);
    assert_eq!(
        player
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::IRON_AXE.id)),
        0,
        "rejected clicked transform reported a successful item use"
    );
    let sounds = observer
        .take_packets()
        .into_iter()
        .filter(|packet| {
            packet.as_ref().get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SOUND
                    .to_id(pumpkin_data::packet::CURRENT_MC_VERSION)
        })
        .count();
    assert_eq!(
        sounds, 0,
        "rejected clicked transform played its wax-removal sound"
    );
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_review_java_hoe_and_shovel_rejections_keep_replacement_without_loot_or_wear() {
    let mut outcomes = Vec::new();
    for (block, item) in [
        (&Block::ROOTED_DIRT, &Item::IRON_HOE),
        (&Block::GRASS_BLOCK, &Item::IRON_SHOVEL),
        (&Block::CAMPFIRE, &Item::IRON_SHOVEL),
    ] {
        let (_directory, server, world, player) = fixture();
        world.set_block_state(&CLICKED, block.default_state.id, BlockFlags::FORCE_STATE);
        player
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, item));
        let called = replace_before_write(&world, CLICKED, &Block::DIAMOND_BLOCK);

        use_on(&player, &server);

        let drops = world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_item_entity().is_some())
            .count();
        outcomes.push((
            block.name,
            called.load(Ordering::Relaxed),
            world.get_block(&CLICKED).id,
            damage(&player),
            player
                .player
                .get_stat(StatisticCategory::Used, i32::from(item.id)),
            drops,
        ));
        world.level.shutdown().await.unwrap();
    }
    assert_eq!(
        outcomes,
        vec![
            ("rooted_dirt", true, Block::DIAMOND_BLOCK.id, 0, 0, 0),
            ("grass_block", true, Block::DIAMOND_BLOCK.id, 0, 0, 0),
            ("campfire", true, Block::DIAMOND_BLOCK.id, 0, 0, 0),
        ],
        "rejected tool use must preserve replacement, wear, statistics and drops"
    );
}
