use std::{
    any::Any,
    cell::RefCell,
    sync::{Arc, Barrier, Mutex, mpsc},
};

use pumpkin_data::{Block, biome::Biome, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{
    Clearable, Inventory, generic_container_screen_handler::create_generic_9x3,
};
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};

use crate::{
    block::entities::chest::ChestBlockEntity,
    entity::{
        Entity, EntityBase, death_test_world::DeathTestWorld, item::ItemEntity,
        vehicle::minecart::MinecartEntity,
    },
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support,
};

thread_local! {
    static CHEST_READS: RefCell<Option<(usize, Arc<Barrier>)>> = const { RefCell::new(None) };
}

// The adapter preserves the actual chest storage and its canonical transaction identity.
// Only the third source read (quick_move's snapshot, after packet validation) is paused.
struct PausedChest(Arc<dyn Inventory>);
impl Clearable for PausedChest {
    fn clear(&self) {
        self.0.clear();
    }
}
impl Inventory for PausedChest {
    fn size(&self) -> usize {
        self.0.size()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn get_stack(&self, slot: usize) -> ItemStack {
        let stack = self.0.get_stack(slot);
        let pause = CHEST_READS.with(|state| {
            let mut state = state.borrow_mut();
            let (reads, barrier) = state.as_mut()?;
            *reads += 1;
            (*reads == 3).then(|| barrier.clone())
        });
        if let Some(pause) = pause {
            pause.wait();
        }
        stack
    }
    fn remove_stack(&self, slot: usize) -> ItemStack {
        self.0.remove_stack(slot)
    }
    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        self.0.remove_stack_specific(slot, amount)
    }
    fn set_stack(&self, slot: usize, stack: ItemStack) {
        self.0.set_stack(slot, stack);
    }
    fn update_slot(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        self.0.update_slot(slot, update);
    }
    fn slot_transaction(&self, slot: usize) -> Arc<Mutex<()>> {
        self.0.slot_transaction(slot)
    }
    fn mark_dirty(&self) {
        self.0.mark_dirty();
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn click() -> SClickSlot {
    SClickSlot {
        sync_id: VarInt(1),
        revision: VarInt(0),
        slot: 0,
        button: 0,
        mode: SlotActionType::QuickMove,
        length_of_array: VarInt(0),
        array_of_changed_slots: Vec::new(),
        carried_item: OptionalItemStackHash(None),
    }
}

fn open_chest(player: &TestPlayer, inventory: Arc<dyn Inventory>) {
    let handler = create_generic_9x3(
        1,
        &player.player.inventory,
        inventory,
        player.player.as_ref(),
    );
    *player.player.current_screen_handler.lock().unwrap() = Arc::new(Mutex::new(handler));
}

fn diamonds(inventory: &dyn Inventory) -> u32 {
    (0..inventory.size())
        .map(|slot| {
            let stack = inventory.get_stack(slot);
            if stack.item == &Item::DIAMOND {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

fn leave_space(player: &TestPlayer, space: u8) {
    for slot in 0..36 {
        player
            .player
            .inventory
            .set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    player
        .player
        .inventory
        .set_stack(8, ItemStack::new(64 - space, &Item::DIAMOND));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_two_packets_withdraw_one_shared_chest_stack() {
    let fixture = DeathTestWorld::new().await;
    let first = TestPlayer::new(&fixture.world());
    let second = TestPlayer::new(&fixture.world());
    // A partial first transfer must leave items available to the other viewer.
    leave_space(&first, 16);
    let world = fixture.world();
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let position = BlockPos::new(8, 64, 8);
    let mut saved = pumpkin_nbt::compound::NbtCompound::new();
    saved.put_string("id", ChestBlockEntity::ID.to_owned());
    saved.put_int("x", position.0.x);
    saved.put_int("y", position.0.y);
    saved.put_int("z", position.0.z);
    let mut saved_stack = pumpkin_nbt::compound::NbtCompound::new();
    saved_stack.put_byte("Slot", 0);
    // ItemStack.MAP_CODEC and ContainerHelper.saveAllItems, also written by older fork builds.
    saved_stack.put_string("id", "minecraft:diamond".into());
    saved_stack.put_int("count", 64);
    saved_stack.put_compound("components", pumpkin_nbt::compound::NbtCompound::new());
    saved.put_list(
        "Items",
        vec![pumpkin_nbt::tag::NbtTag::Compound(saved_stack)],
    );
    // Vanilla and older fork saves enter through the real pending chunk-NBT loader.
    world.add_block_entity_nbt(position, &saved);
    let loaded = world.get_block_entity(&position).unwrap();
    let chest = loaded.clone().get_inventory().unwrap();
    let shared: Arc<dyn Inventory> = Arc::new(PausedChest(chest.clone()));
    open_chest(&first, shared.clone());
    open_chest(&second, shared);
    let barrier = Arc::new(Barrier::new(2));
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        for player in [&first, &second] {
            let barrier = barrier.clone();
            let runtime = runtime.clone();
            let server = &fixture.server;
            scope.spawn(move || {
                let _runtime = runtime.enter();
                CHEST_READS.with(|state| *state.borrow_mut() = Some((0, barrier)));
                player.player.on_slot_click(click(), server);
                CHEST_READS.with(|state| *state.borrow_mut() = None);
            });
        }
    });
    let outcome = (
        diamonds(first.player.inventory.as_ref()) - 48,
        diamonds(second.player.inventory.as_ref()),
        diamonds(chest.as_ref()),
    );
    assert_eq!(outcome.0 + outcome.1 + outcome.2, 64);
    assert!(
        matches!(outcome, (16, 48, 0) | (0, 64, 0)),
        "nonserial outcome: {outcome:?}"
    );
    fixture.server.shutdown().await;
}

fn item_world(fixture: &DeathTestWorld) -> (TestPlayer, Arc<ItemEntity>, Arc<MinecartEntity>) {
    let world = fixture.world();
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    world
        .active_chunks
        .write()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 8.0));
    let item = Arc::new(ItemEntity::new_with_velocity(
        Entity::new(
            world.clone(),
            Vector3::new(8.0, 64.8, 8.0),
            &EntityType::ITEM,
        ),
        ItemStack::new(64, &Item::DIAMOND),
        Vector3::new(0.0, 0.0, 0.0),
        0,
    ));
    item.init_data_tracker();
    world.add_entity_silent(item.clone());
    // The cart is ticked explicitly, outside the world's entity snapshot.
    let cart = Arc::new(MinecartEntity::new(Entity::new(
        world,
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::HOPPER_MINECART,
    )));
    (player, item, cart)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_player_world_tick_vs_hopper_cart_tick() {
    let fixture = DeathTestWorld::new().await;
    let (player, item, cart) = item_world(&fixture);
    leave_space(&player, 4);
    let (ready, wait_ready) = mpsc::channel();
    let (done, wait_done) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        let fixture = &fixture;
        let cart = &cart;
        scope.spawn(move || {
            let _runtime = runtime.enter();
            wait_ready.recv().unwrap();
            cart.tick(cart.as_ref(), &fixture.server);
            done.send(()).unwrap();
        });
        item.set_transfer_pause(Box::new(move || {
            ready.send(()).unwrap();
            wait_done.recv().unwrap();
        }));
        // World::tick drives the real ItemEntity tick and nearby-player collision path.
        fixture.world().tick(&fixture.server);
    });
    let remaining = u32::from(item.get_item_stack().lock().unwrap().item_count);
    let outcome = (
        diamonds(player.player.inventory.as_ref()) - 60,
        diamonds(cart.container().unwrap().as_ref()),
        remaining,
    );
    assert_eq!(outcome, (0, 64, 0));
    assert_eq!(outcome.0 + outcome.1 + outcome.2, 64);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_failed_hopper_cart_cannot_restore_player_pickup() {
    let fixture = DeathTestWorld::new().await;
    let (player, item, cart) = item_world(&fixture);
    leave_space(&player, 4);
    let inventory = cart.container().unwrap();
    for slot in 0..inventory.size() {
        inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    let (ready, wait_ready) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        let item = &item;
        let cart = &cart;
        let fixture = &fixture;
        scope.spawn(move || {
            let _runtime = runtime.enter();
            item.set_transfer_pause(Box::new(move || {
                ready.send(()).unwrap();
                wait_resume.recv().unwrap();
            }));
            cart.tick(cart.as_ref(), &fixture.server);
        });
        wait_ready.recv().unwrap();
        fixture.world().tick(&fixture.server);
        resume.send(()).unwrap();
    });
    let outcome = (
        diamonds(player.player.inventory.as_ref()) - 60,
        diamonds(inventory.as_ref()),
        u32::from(item.get_item_stack().lock().unwrap().item_count),
    );
    assert_eq!(outcome, (4, 0, 60));
    assert_eq!(outcome.0 + outcome.1 + outcome.2, 64);
    assert!(!item.get_entity().is_removed());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_failed_container_transfer_refunds_only_one_item() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(&world);
    let chest = Arc::new(ChestBlockEntity::new(BlockPos::new(8, 65, 8)));
    chest.set_stack(0, ItemStack::new(64, &Item::DIAMOND));
    world.add_block_entity(chest.clone());
    open_chest(&player, chest.clone());
    let cart = Arc::new(MinecartEntity::new(Entity::new(
        world,
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::HOPPER_MINECART,
    )));
    let inventory = cart.container().unwrap();
    for slot in 0..inventory.size() {
        inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    inventory.set_stack(4, ItemStack::new(63, &Item::DIAMOND));
    let filler = TestPlayer::new(&fixture.world());
    filler
        .player
        .inventory
        .set_stack(9, ItemStack::new(1, &Item::DIAMOND));
    let handler = pumpkin_inventory::generic_container_screen_handler::create_hopper(
        1,
        &filler.player.inventory,
        inventory.clone(),
        filler.player.as_ref(),
    );
    *filler.player.current_screen_handler.lock().unwrap() = Arc::new(Mutex::new(handler));
    let (ready, wait_ready) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    cart.set_container_transfer_pause(Box::new(move || {
        ready.send(()).unwrap();
        wait_resume.recv().unwrap();
    }));
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _runtime = runtime.enter();
            cart.tick(cart.as_ref(), &fixture.server);
        });
        wait_ready.recv().unwrap();
        let mut packet = click();
        packet.mode = SlotActionType::Pickup;
        packet.button = SClickSlot::BUTTON_RIGHT;
        // Taking half of 63 or 64 both takes 32: the serial outcome is unambiguous.
        player.player.on_slot_click(packet, &fixture.server);
        let mut pick_up = click();
        pick_up.mode = SlotActionType::Pickup;
        pick_up.slot = 5;
        filler.player.on_slot_click(pick_up, &fixture.server);
        let mut fill = click();
        fill.mode = SlotActionType::Pickup;
        fill.slot = 4;
        filler.player.on_slot_click(fill, &fixture.server);
        resume.send(()).unwrap();
    });
    let screen = player.player.current_screen_handler.lock().unwrap().clone();
    let cursor = screen
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap()
        .item_count;
    assert_eq!(cursor, 32);
    assert_eq!(diamonds(chest.as_ref()), 32);
    assert_eq!(diamonds(inventory.as_ref()), 64);
    assert_eq!(diamonds(filler.player.inventory.as_ref()), 0);
    let filler_screen = filler.player.current_screen_handler.lock().unwrap().clone();
    assert!(
        filler_screen
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        u32::from(cursor) + diamonds(chest.as_ref()) + diamonds(inventory.as_ref()),
        128
    );
    fixture.server.shutdown().await;
}
