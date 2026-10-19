use std::sync::Arc;

use crate::{
    entity::{EntityBase, item::ItemEntity},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_interact_event::PlayerInteractEvent,
    },
    server::{Server, combat_test_support},
    world::{BlockFlags, spawn_test_support},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{Axis, CampfireLikeProperties, PaleOakWoodLikeProperties},
    data_component_impl::BlocksAttacksImpl,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{GameMode, Hand, math::position::BlockPos, math::vector3::Vector3};

fn fixture() -> (
    tempfile::TempDir,
    Arc<Server>,
    Arc<crate::world::World>,
    TestPlayer,
) {
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
    player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);

    (directory, server, world, player)
}

fn put_block(world: &Arc<crate::world::World>, position: BlockPos, block: &'static Block) {
    world.set_block_state(&position, block.default_state.id, BlockFlags::NOTIFY_ALL);
}

fn hold(player: &TestPlayer, hand: Hand, item: &'static Item) {
    player
        .player
        .inventory()
        .set_stack_in_hand(hand, ItemStack::new(1, item));
}

fn damage(player: &TestPlayer, hand: Hand) -> i32 {
    player
        .player
        .inventory()
        .get_stack_in_hand(hand)
        .get_damage()
}

fn use_on(
    player: &TestPlayer,
    server: &Arc<Server>,
    position: BlockPos,
    hand: Hand,
    face: i32,
    sequence: i32,
) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(i32::from(hand == Hand::Left)),
                position,
                face: VarInt(face),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(sequence),
            },
            server,
        )
        .unwrap();
}

fn set_air_above(world: &Arc<crate::world::World>, position: BlockPos) {
    put_block(world, position.up(), &Block::AIR);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_axe_strips_oak_log_and_preserves_axis_in_both_hands() {
    let (_directory, server, world, player) = fixture();

    for (index, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
        let position = BlockPos::new((index as i32) * 2, 64, 1);
        let mut props = PaleOakWoodLikeProperties::default(&Block::OAK_LOG);
        props.r#axis = Axis::X;
        world.set_block_state(
            &position,
            props.to_state_id(&Block::OAK_LOG),
            BlockFlags::NOTIFY_ALL,
        );
        hold(&player, hand, &Item::IRON_AXE);

        use_on(&player, &server, position, hand, 1, index as i32 + 1);

        let stripped = world.get_block_state_id(&position);
        assert_eq!(Block::from_state_id(stripped), &Block::STRIPPED_OAK_LOG);
        assert_eq!(
            PaleOakWoodLikeProperties::from_state_id(stripped).r#axis,
            Axis::X
        );
        assert_eq!(
            damage(&player, hand),
            1,
            "successful iron axes lose one durability"
        );
    }

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_axe_scrapes_exposed_copper_in_both_hands() {
    let (_directory, server, world, player) = fixture();

    for (index, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
        let position = BlockPos::new((index as i32) * 2, 64, 1);
        put_block(&world, position, &Block::EXPOSED_COPPER);
        hold(&player, hand, &Item::IRON_AXE);

        use_on(&player, &server, position, hand, 1, index as i32 + 1);

        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&position)),
            &Block::COPPER_BLOCK
        );
        assert_eq!(
            damage(&player, hand),
            1,
            "successful iron axes lose one durability"
        );
    }

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_axe_removes_copper_wax_in_both_hands() {
    let (_directory, server, world, player) = fixture();

    for (index, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
        let position = BlockPos::new((index as i32) * 2, 64, 1);
        put_block(&world, position, &Block::WAXED_EXPOSED_COPPER);
        hold(&player, hand, &Item::IRON_AXE);

        use_on(&player, &server, position, hand, 1, index as i32 + 1);

        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&position)),
            &Block::EXPOSED_COPPER
        );
        assert_eq!(
            damage(&player, hand),
            1,
            "successful iron axes lose one durability"
        );
    }

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_hoe_tills_dirt_in_both_hands() {
    let (_directory, server, world, player) = fixture();

    for (index, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
        let position = BlockPos::new((index as i32) * 2, 64, 1);
        put_block(&world, position, &Block::DIRT);
        set_air_above(&world, position);
        hold(&player, hand, &Item::IRON_HOE);

        use_on(&player, &server, position, hand, 1, index as i32 + 1);

        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&position)),
            &Block::FARMLAND
        );
        assert_eq!(
            damage(&player, hand),
            1,
            "successful iron hoes lose one durability"
        );
    }

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_hoe_tills_rooted_dirt_and_drops_hanging_roots() {
    let (_directory, server, world, player) = fixture();

    for (index, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
        let position = BlockPos::new((index as i32) * 2, 64, 1);
        put_block(&world, position, &Block::ROOTED_DIRT);
        hold(&player, hand, &Item::IRON_HOE);

        use_on(&player, &server, position, hand, 1, index as i32 + 1);

        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&position)),
            &Block::DIRT
        );
        assert_eq!(
            damage(&player, hand),
            1,
            "successful iron hoes lose one durability"
        );
        let roots: u32 = world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.cast_any().downcast_ref::<ItemEntity>())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == &Item::HANGING_ROOTS)
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert_eq!(
            roots,
            index as u32 + 1,
            "rooted dirt yields one hanging root per use"
        );
    }

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_shovel_creates_grass_paths_in_both_hands() {
    let (_directory, server, world, player) = fixture();

    for (index, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
        let position = BlockPos::new((index as i32) * 2, 64, 1);
        put_block(&world, position, &Block::GRASS_BLOCK);
        set_air_above(&world, position);
        hold(&player, hand, &Item::IRON_SHOVEL);

        use_on(&player, &server, position, hand, 1, index as i32 + 1);

        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&position)),
            &Block::DIRT_PATH
        );
        assert_eq!(
            damage(&player, hand),
            1,
            "successful iron shovels lose one durability"
        );
    }

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_shovel_extinguishes_campfires_with_survival_durability_only() {
    let (_directory, server, world, player) = fixture();

    let survival_position = BlockPos::new(0, 64, 1);
    let mut survival_lit = CampfireLikeProperties::default(&Block::CAMPFIRE);
    survival_lit.lit = true;
    world.set_block_state(
        &survival_position,
        survival_lit.to_state_id(&Block::CAMPFIRE),
        BlockFlags::NOTIFY_ALL,
    );
    hold(&player, Hand::Right, &Item::IRON_SHOVEL);
    use_on(&player, &server, survival_position, Hand::Right, 1, 1);
    assert!(
        !CampfireLikeProperties::from_state_id(world.get_block_state_id(&survival_position)).lit
    );
    assert_eq!(damage(&player, Hand::Right), 1);

    let creative_position = BlockPos::new(1, 64, 1);
    let mut creative_lit = CampfireLikeProperties::default(&Block::SOUL_CAMPFIRE);
    creative_lit.lit = true;
    world.set_block_state(
        &creative_position,
        creative_lit.to_state_id(&Block::SOUL_CAMPFIRE),
        BlockFlags::NOTIFY_ALL,
    );
    player.player.gamemode.store(GameMode::Creative);
    hold(&player, Hand::Right, &Item::IRON_SHOVEL);
    use_on(&player, &server, creative_position, Hand::Right, 1, 2);
    assert!(
        !CampfireLikeProperties::from_state_id(world.get_block_state_id(&creative_position)).lit
    );
    assert_eq!(damage(&player, Hand::Right), 0);

    let unlit_position = BlockPos::new(2, 64, 1);
    let mut unlit = CampfireLikeProperties::default(&Block::CAMPFIRE);
    unlit.lit = false;
    let unlit_state = unlit.to_state_id(&Block::CAMPFIRE);
    world.set_block_state(&unlit_position, unlit_state, BlockFlags::NOTIFY_ALL);
    player.player.gamemode.store(GameMode::Survival);
    hold(&player, Hand::Right, &Item::IRON_SHOVEL);
    use_on(&player, &server, unlit_position, Hand::Right, 1, 3);
    assert_eq!(
        world.get_block_state_id(&unlit_position),
        unlit_state,
        "an unlit campfire is the no-op negative control"
    );
    assert_eq!(damage(&player, Hand::Right), 0);

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_tool_transform_controls_leave_blocks_and_tools_unchanged() {
    let (_directory, server, world, player) = fixture();

    let unrelated = BlockPos::new(0, 64, 1);
    put_block(&world, unrelated, &Block::STONE);
    hold(&player, Hand::Right, &Item::IRON_AXE);
    use_on(&player, &server, unrelated, Hand::Right, 1, 1);
    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&unrelated)),
        &Block::STONE
    );
    assert_eq!(damage(&player, Hand::Right), 0);

    let face = BlockPos::new(1, 64, 1);
    put_block(&world, face, &Block::DIRT);
    set_air_above(&world, face);
    hold(&player, Hand::Right, &Item::IRON_HOE);
    use_on(&player, &server, face, Hand::Right, 0, 2);
    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&face)),
        &Block::DIRT
    );
    assert_eq!(
        damage(&player, Hand::Right),
        0,
        "hoe rejects its disallowed down face"
    );

    let blocked_farmland = BlockPos::new(2, 64, 1);
    put_block(&world, blocked_farmland, &Block::DIRT);
    put_block(&world, blocked_farmland.up(), &Block::STONE);
    use_on(&player, &server, blocked_farmland, Hand::Right, 1, 3);
    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&blocked_farmland)),
        &Block::DIRT
    );
    assert_eq!(
        damage(&player, Hand::Right),
        0,
        "solid block obstructs farmland"
    );

    let blocked_path = BlockPos::new(3, 64, 1);
    put_block(&world, blocked_path, &Block::GRASS_BLOCK);
    put_block(&world, blocked_path.up(), &Block::STONE);
    hold(&player, Hand::Right, &Item::IRON_SHOVEL);
    use_on(&player, &server, blocked_path, Hand::Right, 1, 4);
    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&blocked_path)),
        &Block::GRASS_BLOCK
    );
    assert_eq!(
        damage(&player, Hand::Right),
        0,
        "solid block obstructs a dirt path"
    );

    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_offhand_shield_blocks_mainhand_transform_intent() {
    let (_directory, server, world, player) = fixture();
    let position = BlockPos::new(0, 64, 1);
    put_block(&world, position, &Block::OAK_LOG);
    hold(&player, Hand::Right, &Item::IRON_AXE);
    hold(&player, Hand::Left, &Item::SHIELD);
    assert!(
        player
            .player
            .inventory()
            .off_hand_item()
            .get_data_component::<BlocksAttacksImpl>()
            .is_some()
    );

    use_on(&player, &server, position, Hand::Right, 1, 1);

    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&position)),
        &Block::OAK_LOG
    );
    assert_eq!(damage(&player, Hand::Right), 0);
    world.level.shutdown().await.unwrap();
}

struct CancelBlockUse;

impl EventHandler<PlayerInteractEvent> for CancelBlockUse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            event.cancelled = true;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g04_java_cancelled_interact_callback_prevents_transform() {
    let (_directory, server, world, player) = fixture();
    let position = BlockPos::new(0, 64, 1);
    put_block(&world, position, &Block::OAK_LOG);
    hold(&player, Hand::Right, &Item::IRON_AXE);
    server.plugin_manager.register::<PlayerInteractEvent, _>(
        Arc::new(CancelBlockUse),
        EventPriority::Normal,
        true,
    );

    use_on(&player, &server, position, Hand::Right, 1, 1);

    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&position)),
        &Block::OAK_LOG
    );
    assert_eq!(damage(&player, Hand::Right), 0);
    world.level.shutdown().await.unwrap();
}
