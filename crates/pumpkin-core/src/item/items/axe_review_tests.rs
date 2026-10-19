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
        ChestLikeProperties, ChestType, DoubleBlockHalf, HorizontalFacing, OakDoorLikeProperties,
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
    atomic::{AtomicBool, Ordering},
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
