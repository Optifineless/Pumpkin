use std::sync::Arc;

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
    entity::{death_test_world::DeathTestWorld, item::ItemEntity},
    inventory_transaction_tests::{click, open_chest},
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
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
