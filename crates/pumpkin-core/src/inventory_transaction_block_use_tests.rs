use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockDirection, data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{
        BlockBehaviour, BlockHitResult, UseWithItemArgs,
        entities::{
            BlockEntity, campfire::CampfireBlockEntity, decorated_pot::DecoratedPotBlockEntity,
        },
    },
    entity::{EntityBase, death_test_world::DeathTestWorld, item::ItemEntity},
    inventory_transaction_tests::{click, open_chest},
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority},
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::ItemStackSerializer,
    java::{
        client::play::{CBlockEntityData, CBlockUpdate, CSetPlayerInventory},
        server::play::SUseItemOn,
    },
};

fn storage(entity: &dyn BlockEntity) -> &dyn Inventory {
    if let Some(pot) = entity.as_any().downcast_ref::<DecoratedPotBlockEntity>() {
        pot
    } else {
        entity
            .as_any()
            .downcast_ref::<CampfireBlockEntity>()
            .unwrap()
    }
}

fn total(inventory: &dyn Inventory) -> u32 {
    (0..inventory.size())
        .map(|slot| u32::from(inventory.get_stack(slot).item_count))
        .sum()
}

fn dropped(world: &crate::world::World, item_type: &Item) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            entity
                .cast_any()
                .downcast_ref::<ItemEntity>()
                .map(|item| item.get_item_stack().lock().unwrap().clone())
        })
        .filter(|stack| stack.item == item_type)
        .map(|stack| u32::from(stack.item_count))
        .sum()
}

fn legacy_food(
    fixture: &DeathTestWorld,
    block: &Block,
    item: &Item,
    count: u8,
) -> Arc<dyn BlockEntity> {
    let world = fixture.world();
    let position = BlockPos::new(8, 65, 8);
    // LevelChunk loads block states before pending NBT; placement would publish an empty live BE.
    let relative = position.chunk_relative_position();
    world
        .level
        .read_chunk_sync(&position.chunk_position(), |chunk| {
            chunk
                .set_block_absolute_y_if(
                    relative.x as usize,
                    relative.y,
                    relative.z as usize,
                    block.default_state.id,
                    |_| true,
                )
                .unwrap();
        })
        .unwrap();
    assert!(world.get_block_entity(&position).is_none());
    let mut saved = NbtCompound::new();
    let mut food = NbtCompound::new();
    food.put_string("id", item.registry_key.to_owned());
    food.put_int("count", i32::from(count));
    food.put_compound("components", NbtCompound::new());
    // DecoratedPotBlockEntity.saveAdditional / CampfireBlockEntity.saveAdditional, also used by old fork builds.
    if block == &Block::DECORATED_POT {
        saved.put_string("id", DecoratedPotBlockEntity::ID.to_owned());
        saved.put_compound("item", food);
    } else {
        saved.put_string("id", CampfireBlockEntity::ID.to_owned());
        food.put_byte("Slot", 0);
        saved.put_list("Items", vec![NbtTag::Compound(food)]);
        saved.put("CookingTimes", NbtTag::IntArray(vec![1, 0, 0, 0]));
        saved.put("CookingTotalTimes", NbtTag::IntArray(vec![2, 0, 0, 0]));
    }
    saved.put_int("x", position.0.x);
    saved.put_int("y", position.0.y);
    saved.put_int("z", position.0.z);
    world.add_block_entity_nbt(position, &saved);
    let loaded = world.get_block_entity(&position).unwrap();
    assert_eq!(total(storage(loaded.as_ref())), u32::from(count));
    loaded
}

fn set_spawn(fixture: &DeathTestWorld, position: BlockPos) {
    fixture.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.spawn_x = position.0.x;
        next.spawn_y = position.0.y;
        next.spawn_z = position.0.z;
        next
    });
}

fn add_unrelated_operator(fixture: &DeathTestWorld) {
    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ops
        .push(pumpkin_config::op::Op::new(
            uuid::Uuid::new_v4(),
            "spawn-protection-test-op".to_owned(),
            pumpkin_util::PermissionLvl::Zero,
            false,
        ));
}

struct CountPlayerInteract(Arc<AtomicUsize>);

impl EventHandler<crate::plugin::api::events::player::player_interact_event::PlayerInteractEvent>
    for CountPlayerInteract
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut crate::plugin::api::events::player::player_interact_event::PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_protection_matches_vanilla_operator_membership_radius_and_dimension() {
    use pumpkin_data::dimension::Dimension;
    use pumpkin_util::{math::position::BlockPos, permission::PermissionLvl};

    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Zero);
    let spawn = BlockPos::new(8, 64, 8);
    set_spawn(&fixture, spawn);
    let inside = BlockPos::new(24, 90, 24);
    let outside = BlockPos::new(25, 90, 8);

    assert!(!world.is_in_spawn_protection(&player.player, &inside));
    add_unrelated_operator(&fixture);
    assert!(world.is_in_spawn_protection(&player.player, &inside));
    assert!(!world.is_in_spawn_protection(&player.player, &outside));

    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ops
        .push(pumpkin_config::op::Op::new(
            player.player.gameprofile.id,
            player.player.gameprofile.name.clone(),
            PermissionLvl::Zero,
            false,
        ));
    assert!(!world.is_in_spawn_protection(&player.player, &inside));

    let nether = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    assert!(!nether.is_in_spawn_protection(&player.player, &inside));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_protection_denies_java_decorated_pot_deposit_before_callbacks()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::plugin::api::events::player::player_interact_event::PlayerInteractEvent;
    use pumpkin_data::biome::Biome;
    use pumpkin_util::math::vector3::Vector3;

    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pot = legacy_food(&fixture, &Block::DECORATED_POT, &Item::DIAMOND, 0);
    let position = pot.get_position();
    set_spawn(&fixture, position);
    add_unrelated_operator(&fixture);

    let mut client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 9.5));
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    client.player.inventory().set_stack(0, diamond.clone());

    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(
            Arc::new(CountPlayerInteract(calls.clone())),
            EventPriority::Normal,
            true,
        );
    client.client().handle_use_item_on(
        &client.player,
        &SUseItemOn {
            hand: VarInt(0),
            position,
            face: VarInt(1),
            cursor_pos: Vector3::new(0.5, 0.5, 0.5),
            inside_block: false,
            is_against_world_border: false,
            sequence: VarInt(42),
        },
        &fixture.server,
    )?;

    assert_eq!(total(storage(pot.as_ref())), 0);
    assert!(client.player.inventory().held_item().are_equal(&diamond));
    assert_eq!(calls.load(Relaxed), 0);
    assert_eq!(client.client().packet_sequence.load(Relaxed), 42);
    let packets = client.take_packets();
    let adjacent = position.offset(BlockDirection::Up.to_offset());
    for corrected in [position, adjacent] {
        let state = world.get_block_state_id(&corrected);
        let expected = client.client().serialize_packet(&CBlockUpdate::new(
            corrected,
            VarInt(i32::from(state.as_u16())),
        ))?;
        assert!(packets.contains(&expected));
    }
    let Some(nbt) = pot.chunk_data_nbt() else {
        return Err(std::io::Error::other("decorated pot has no update NBT").into());
    };
    let block_entity_update = CBlockEntityData::new(
        position,
        VarInt(pot.get_id() as i32),
        pumpkin_nbt::Nbt::from(nbt).write_unnamed().as_ref().into(),
    );
    assert!(packets.contains(&client.client().serialize_packet(&block_entity_update)?));
    let slot = ItemStackSerializer::from(diamond);
    let hand_update = CSetPlayerInventory::new(VarInt(0), &slot);
    assert!(packets.contains(&client.client().serialize_packet(&hand_update)?));
    fixture.server.shutdown().await;
    Ok(())
}

fn use_food(
    fixture: &DeathTestWorld,
    block: &Block,
    behaviour: &dyn BlockBehaviour,
    player: &TestPlayer,
    held: &mut ItemStack,
) {
    behaviour.use_with_item(UseWithItemArgs {
        server: &fixture.server,
        world: &fixture.world(),
        block,
        position: &BlockPos::new(8, 65, 8),
        player: &player.player,
        hit: &BlockHitResult {
            face: &BlockDirection::Up,
            cursor_pos: &Vector3::new(0.5, 0.5, 0.5),
        },
        item_stack: held,
        equipment_slot: &EquipmentSlot::MAIN_HAND,
    });
}

async fn stale_hand_insertion(
    block: &'static Block,
    item: &'static Item,
    behaviour: &dyn BlockBehaviour,
    count: u8,
) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(
        &world,
        proto(&pumpkin_data::biome::Biome::PLAINS, &Block::STONE),
    );
    let destination = legacy_food(&fixture, block, item, count);
    let position = destination.get_position();
    let inventory = storage(destination.as_ref());
    let player = TestPlayer::new(&world);
    player
        .player
        .inventory
        .set_stack(0, ItemStack::new(64, item));
    let mut held = player.player.inventory.get_stack(0);
    let chest = Arc::new(crate::block::entities::chest::ChestBlockEntity::new(
        position.east(),
    ));
    open_chest(&player, chest.clone());
    let mut packet = click();
    packet.slot = 54;
    player.player.on_slot_click(packet, &fixture.server);
    assert_eq!(chest.get_stack(0).item_count, 64);
    // ServerPlayerGameMode.useItemOn delegates the previously observed hand to this entry point.
    use_food(&fixture, block, behaviour, &player, &mut held);
    assert_eq!(total(inventory), u32::from(count));
    assert_eq!(total(chest.as_ref()), 64);
    assert!(player.player.inventory.is_empty());
    player
        .player
        .inventory
        .set_stack(0, ItemStack::new(2, item));
    held = player.player.inventory.get_stack(0);
    use_food(&fixture, block, behaviour, &player, &mut held);
    assert_eq!(player.player.inventory.get_stack(0).item_count, 1);
    assert_eq!(total(inventory), u32::from(count) + 1);
    let mut saved = NbtCompound::new();
    destination.write_nbt(&mut saved);
    if block == &Block::DECORATED_POT {
        assert_eq!(
            saved.get_compound("item").unwrap().get_int("count"),
            Some(8)
        );
    } else {
        assert_eq!(saved.get_int_array("CookingTimes").unwrap()[0], 1);
        destination.tick(&world);
        assert!(inventory.get_stack(0).is_empty());
        assert_eq!(total(inventory), 1);
    }
    world.break_block(&position, None, BlockFlags::empty());
    assert!(inventory.is_empty());
    assert!(!inventory.slot_transaction(0).is_live());
    if block == &Block::DECORATED_POT {
        assert_eq!(dropped(&world, &Item::DIAMOND), 8);
    } else {
        // Vanilla's porkchop_from_campfire_cooking recipe produces one cooked porkchop.
        assert_eq!(dropped(&world, &Item::PORKCHOP), 1);
        assert_eq!(dropped(&world, &Item::COOKED_PORKCHOP), 1);
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_pot_rejects_a_hand_moved_by_a_real_click() {
    stale_hand_insertion(
        &Block::DECORATED_POT,
        &Item::DIAMOND,
        &crate::block::blocks::decorated_pot::DecoratedPotBlock,
        7,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_campfire_rejects_a_hand_moved_by_a_real_click() {
    stale_hand_insertion(
        &Block::CAMPFIRE,
        &Item::PORKCHOP,
        &crate::block::blocks::campfire::CampfireBlock,
        1,
    )
    .await;
}
