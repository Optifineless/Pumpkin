use std::sync::Arc;

use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{BlockFlags, spawn_test_support},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{
        ChestLikeProperties, ChestType, DoorHinge, DoubleBlockHalf, HorizontalFacing,
        OakDoorLikeProperties,
    },
    data_component_impl::LockImpl,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{Hand, math::position::BlockPos, math::vector3::Vector3};
use pumpkin_world::chunk::io::{FileIO, LoadedData};

const LOOT_TABLE: &str = "minecraft:chests/simple_dungeon";
const LOOT_SEED: i64 = 0x227_2026;

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

fn hold(player: &TestPlayer, item: &'static Item) {
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, item));
}

fn use_on(player: &TestPlayer, server: &Arc<Server>, position: BlockPos, sequence: i32) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(sequence),
            },
            server,
        )
        .unwrap();
}

async fn reload_block_entity(world: &Arc<crate::world::World>, position: BlockPos) {
    let chunk_pos = position.chunk_position();
    world.save().await;
    let chunk = world.level.loaded_chunks.get(&chunk_pos).unwrap().clone();
    world
        .level
        .chunk_saver
        .save_chunks(&world.level.level_folder, vec![(chunk_pos, chunk)])
        .await
        .unwrap();

    if let Some(mut entities) = world.block_entities.get_mut(&chunk_pos) {
        entities.remove(&position);
    }
    world
        .block_entities
        .remove_if(&chunk_pos, |_, entities| entities.is_empty());
    world.custom_block_entity_data.remove(&position);
    world.level.loaded_chunks.remove(&chunk_pos);

    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    world
        .level
        .chunk_saver
        .fetch_chunks(&world.level.level_folder, &[chunk_pos], send)
        .await;
    let LoadedData::Loaded(chunk) = receive.recv().await.unwrap() else {
        panic!("saved copper chest chunk missing")
    };
    world.level.loaded_chunks.insert(chunk_pos, chunk);
}

fn pending_chest_nbt(position: BlockPos) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", "minecraft:chest".to_owned());
    nbt.put_int("x", position.0.x);
    nbt.put_int("y", position.0.y);
    nbt.put_int("z", position.0.z);
    nbt.put_string("LootTable", LOOT_TABLE.to_owned());
    nbt.put_long("LootTableSeed", LOOT_SEED);

    let mut lock = NbtCompound::new();
    lock.put_string("minecraft:custom_name", "vault key".to_owned());
    nbt.put("lock", NbtTag::Compound(lock));

    let mut namespace = NbtCompound::new();
    namespace.put_int("marker", 227);
    let mut custom_data = NbtCompound::new();
    custom_data.put_compound("test", namespace);
    nbt.put_compound("PumpkinCustomData", custom_data);
    nbt
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b227_java_axe_preserves_pending_copper_chest_data_through_reload() {
    let (_directory, server, world, player) = fixture();
    let position = BlockPos::new(0, 64, 1);
    world.set_block_state(
        &position,
        Block::WAXED_COPPER_CHEST.default_state.id,
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
    );
    world.add_block_entity_nbt(position, &pending_chest_nbt(position));
    player.player.get_entity().set_sneaking(true);
    hold(&player, &Item::IRON_AXE);

    use_on(&player, &server, position, 1);

    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&position)),
        &Block::COPPER_CHEST
    );
    let chest = world.get_block_entity(&position).unwrap();
    assert!(
        chest.has_loot_table(),
        "deferred loot table must remain pending"
    );
    assert_eq!(
        chest.container_lock(),
        Some(LockImpl {
            predicate: {
                let mut lock = NbtCompound::new();
                lock.put_string("minecraft:custom_name", "vault key".to_owned());
                lock
            }
        })
    );
    assert_eq!(
        world.get_block_entity_custom_data(&position, "test", "marker"),
        Some(NbtTag::Int(227))
    );
    let mut saved = NbtCompound::new();
    chest.write_internal(&mut saved);
    assert_eq!(saved.get_string("LootTable").unwrap(), LOOT_TABLE);
    assert_eq!(saved.get_long("LootTableSeed"), Some(LOOT_SEED));

    reload_block_entity(&world, position).await;

    let reloaded = world.get_block_entity(&position).unwrap();
    assert!(reloaded.has_loot_table());
    let mut saved = NbtCompound::new();
    reloaded.write_internal(&mut saved);
    assert_eq!(saved.get_string("LootTable").unwrap(), LOOT_TABLE);
    assert_eq!(saved.get_long("LootTableSeed"), Some(LOOT_SEED));
    assert_eq!(
        reloaded.container_lock(),
        chest.container_lock(),
        "container lock must survive chunk reload"
    );
    assert_eq!(
        world.get_block_entity_custom_data(&position, "test", "marker"),
        Some(NbtTag::Int(227))
    );
    world.level.shutdown().await.unwrap();
}

fn add_inventory(world: &Arc<crate::world::World>, position: BlockPos, count: u8) {
    world
        .get_block_entity(&position)
        .unwrap()
        .get_inventory()
        .unwrap()
        .set_stack(0, ItemStack::new(count, &Item::DIAMOND));
}

fn assert_inventory(world: &Arc<crate::world::World>, position: BlockPos, count: u8) {
    let inventory = world
        .get_block_entity(&position)
        .unwrap()
        .get_inventory()
        .unwrap();
    let stack = inventory.get_stack(0);
    assert_eq!(stack.item, &Item::DIAMOND);
    assert_eq!(stack.item_count, count);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b227_java_axe_preserves_stocked_single_and_double_copper_chests() {
    let (_directory, server, world, player) = fixture();
    let single = BlockPos::new(0, 64, 1);
    put_block(&world, single, &Block::WAXED_COPPER_CHEST);
    add_inventory(&world, single, 5);

    let left = BlockPos::new(2, 64, 1);
    let right = left.offset(Vector3::new(1, 0, 0));
    let mut left_props = ChestLikeProperties::default(&Block::WAXED_COPPER_CHEST);
    left_props.facing = HorizontalFacing::North;
    left_props.r#type = ChestType::Left;
    let mut right_props = ChestLikeProperties::default(&Block::WAXED_COPPER_CHEST);
    right_props.facing = HorizontalFacing::North;
    right_props.r#type = ChestType::Right;
    world.set_block_state(
        &left,
        left_props.to_state_id(&Block::WAXED_COPPER_CHEST),
        BlockFlags::NOTIFY_ALL,
    );
    world.set_block_state(
        &right,
        right_props.to_state_id(&Block::WAXED_COPPER_CHEST),
        BlockFlags::NOTIFY_ALL,
    );
    add_inventory(&world, left, 7);
    add_inventory(&world, right, 11);

    player.player.get_entity().set_sneaking(true);
    hold(&player, &Item::IRON_AXE);
    use_on(&player, &server, single, 1);
    hold(&player, &Item::IRON_AXE);
    use_on(&player, &server, left, 2);

    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&left)),
        &Block::COPPER_CHEST,
        "the real Java use-on path must transform the clicked half"
    );
    assert_eq!(
        Block::from_state_id(world.get_block_state_id(&right)),
        &Block::COPPER_CHEST,
        "the connected half must follow the clicked chest's stage"
    );
    for (position, half) in [(left, ChestType::Left), (right, ChestType::Right)] {
        let properties = ChestLikeProperties::from_state_id(world.get_block_state_id(&position));
        assert_eq!(properties.r#type, half);
        assert_eq!(properties.facing, HorizontalFacing::North);
    }
    assert_inventory(&world, single, 5);
    assert_inventory(&world, left, 7);
    assert_inventory(&world, right, 11);

    let hopper_inventory =
        crate::block::entities::hopper::test_get_container_at(&world, &left).unwrap();
    assert_eq!(
        hopper_inventory.size(),
        54,
        "the hopper container lookup must combine both inventories"
    );
    world.level.shutdown().await.unwrap();
}

fn put_copper_door_pair(world: &Arc<crate::world::World>, lower: BlockPos) {
    let mut lower_props = OakDoorLikeProperties::default(&Block::WAXED_COPPER_DOOR);
    lower_props.half = DoubleBlockHalf::Lower;
    lower_props.facing = HorizontalFacing::East;
    lower_props.hinge = DoorHinge::Right;
    let mut upper_props = OakDoorLikeProperties::default(&Block::WAXED_COPPER_DOOR);
    upper_props.half = DoubleBlockHalf::Upper;
    upper_props.facing = HorizontalFacing::East;
    upper_props.hinge = DoorHinge::Right;

    let flags = BlockFlags::NOTIFY_ALL
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
        | BlockFlags::UPDATE_KNOWN_SHAPE;
    world.set_block_state(
        &lower,
        lower_props.to_state_id(&Block::WAXED_COPPER_DOOR),
        flags,
    );
    world.set_block_state(
        &lower.up(),
        upper_props.to_state_id(&Block::WAXED_COPPER_DOOR),
        flags,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b227_java_axe_keeps_blocks_above_lower_and_upper_copper_door_uses() {
    let (_directory, server, world, player) = fixture();
    player.player.get_entity().set_sneaking(true);

    for (index, target_half) in [DoubleBlockHalf::Lower, DoubleBlockHalf::Upper]
        .into_iter()
        .enumerate()
    {
        let lower = BlockPos::new((index as i32) * 3, 64, 1);
        put_copper_door_pair(&world, lower);
        let upper = lower.up();
        let valuable = upper.up();
        put_block(&world, valuable, &Block::DIAMOND_BLOCK);
        hold(&player, &Item::IRON_AXE);
        let target = if target_half == DoubleBlockHalf::Lower {
            lower
        } else {
            upper
        };
        use_on(&player, &server, target, index as i32 + 1);

        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&lower)),
            &Block::COPPER_DOOR
        );
        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&upper)),
            &Block::COPPER_DOOR
        );
        assert_eq!(
            OakDoorLikeProperties::from_state_id(world.get_block_state_id(&lower)).half,
            DoubleBlockHalf::Lower
        );
        let lower_props = OakDoorLikeProperties::from_state_id(world.get_block_state_id(&lower));
        let upper_props = OakDoorLikeProperties::from_state_id(world.get_block_state_id(&upper));
        assert_eq!(lower_props.facing, HorizontalFacing::East);
        assert_eq!(lower_props.hinge, DoorHinge::Right);
        assert_eq!(upper_props.half, DoubleBlockHalf::Upper);
        assert_eq!(upper_props.facing, lower_props.facing);
        assert_eq!(upper_props.hinge, lower_props.hinge);
        assert_eq!(
            Block::from_state_id(world.get_block_state_id(&valuable)),
            &Block::DIAMOND_BLOCK,
            "scraping either half must not overwrite the block above the door"
        );
    }

    world.level.shutdown().await.unwrap();
}
