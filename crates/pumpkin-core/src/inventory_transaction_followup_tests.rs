use std::{
    any::Any,
    cell::RefCell,
    sync::{Arc, mpsc},
};

use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{FacingHopper, JukeboxLikeProperties},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::{Clearable, Inventory, inventory::slot_transaction::Transaction};
use pumpkin_protocol::java::server::play::SlotActionType;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::entities::{
        BlockEntity, chest::ChestBlockEntity, hopper::HopperBlockEntity,
        jukebox::JukeboxBlockEntity,
    },
    command::CommandSender,
    entity::{
        Entity, EntityBase, death_test_world::DeathTestWorld, vehicle::minecart::MinecartEntity,
    },
    inventory_transaction_tests::{click, diamonds, open_chest},
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};

type Pause = Box<dyn FnOnce() + Send>;
type ReadPause = (usize, usize, Pause);
thread_local! {
    static READ_PAUSE: RefCell<Option<ReadPause>> = const { RefCell::new(None) };
    static WRITE_PAUSE: RefCell<Option<Pause>> = const { RefCell::new(None) };
    static RETIRE_PAUSE: RefCell<Option<Pause>> = const { RefCell::new(None) };
}

pub fn pause_retirement() {
    let pause = RETIRE_PAUSE.with(|slot| slot.borrow_mut().take());
    if let Some(pause) = pause {
        pause();
    }
}

struct PausedInventory(Arc<dyn Inventory>);
impl Clearable for PausedInventory {
    fn clear(&self) {
        self.0.clear();
    }
}
impl Inventory for PausedInventory {
    fn size(&self) -> usize {
        self.0.size()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn slot_transaction(&self, slot: usize) -> Arc<Transaction> {
        self.0.slot_transaction(slot)
    }
    fn collect_transaction_owners(
        &self,
        owners: &mut pumpkin_inventory::inventory::slot_transaction::Owners,
    ) {
        self.0.collect_transaction_owners(owners);
    }
    fn get_stack(&self, slot: usize) -> ItemStack {
        let stack = self.0.get_stack(slot);
        let pause = READ_PAUSE.with(|state| {
            let mut state = state.borrow_mut();
            let (watched_slot, remaining, _) = state.as_mut()?;
            if slot != *watched_slot {
                return None;
            }
            *remaining -= 1;
            if *remaining == 0 {
                state.take().map(|(_, _, pause)| pause)
            } else {
                None
            }
        });
        if let Some(pause) = pause {
            pause();
        }
        stack
    }
    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        self.0.update_slot_storage(slot, update);
        let pause = WRITE_PAUSE.with(|state| state.borrow_mut().take());
        if let Some(pause) = pause {
            pause();
        }
    }
    fn set_stack(&self, slot: usize, stack: ItemStack) {
        self.0.set_stack(slot, stack);
    }
    fn remove_stack(&self, slot: usize) -> ItemStack {
        self.0.remove_stack(slot)
    }
    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        self.0.remove_stack_specific(slot, amount)
    }
    fn on_slot_changed(&self, slot: usize) {
        self.0.on_slot_changed(slot);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct PausedChest {
    chest: Arc<ChestBlockEntity>,
    inventory: Arc<dyn Inventory>,
}
impl BlockEntity for PausedChest {
    fn write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.chest.write_nbt(nbt);
    }
    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self {
        let chest = Arc::new(ChestBlockEntity::from_nbt(nbt, position));
        Self {
            inventory: Arc::new(PausedInventory(chest.clone())),
            chest,
        }
    }
    fn resource_location(&self) -> &'static str {
        self.chest.resource_location()
    }
    fn get_position(&self) -> BlockPos {
        self.chest.get_position()
    }
    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self.inventory.clone())
    }
    fn on_block_replaced(self: Arc<Self>, world: &Arc<crate::world::World>, position: &BlockPos) {
        self.chest.clone().on_block_replaced(world, position);
    }
    fn as_any(&self) -> &dyn Any {
        self.chest.as_ref()
    }
}

fn chest(fixture: &DeathTestWorld, count: u8) -> Arc<ChestBlockEntity> {
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let position = BlockPos::new(8, 65, 8);
    world.set_block_state(
        &position,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let chest = Arc::new(ChestBlockEntity::new(position));
    chest.set_stack(0, ItemStack::new(count, &Item::DIAMOND));
    world.add_block_entity(chest.clone());
    chest
}

fn cursor(player: &TestPlayer) -> u32 {
    let screen = player.player.current_screen_handler.lock().unwrap().clone();
    u32::from(
        screen
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .item_count,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_number_key_vs_quick_move_packets() {
    let fixture = DeathTestWorld::new().await;
    let chest = chest(&fixture, 64);
    let first = TestPlayer::new(&fixture.world());
    let second = TestPlayer::new(&fixture.world());
    let shared: Arc<dyn Inventory> = Arc::new(PausedInventory(chest.clone()));
    open_chest(&first, shared.clone());
    open_chest(&second, shared);
    let (ready, wait_ready) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _runtime = runtime.enter();
            READ_PAUSE.with(|slot| {
                *slot.borrow_mut() = Some((
                    0,
                    2,
                    Box::new(move || {
                        ready.send(()).unwrap();
                        wait_resume.recv().unwrap();
                    }),
                ));
            });
            let mut packet = click();
            packet.mode = SlotActionType::Swap;
            second.player.on_slot_click(packet, &fixture.server);
        });
        wait_ready.recv().unwrap();
        first.player.on_slot_click(click(), &fixture.server);
        resume.send(()).unwrap();
    });
    assert_eq!(
        diamonds(chest.as_ref())
            + diamonds(first.player.inventory.as_ref())
            + diamonds(second.player.inventory.as_ref()),
        64
    );
    assert_eq!(diamonds(first.player.inventory.as_ref()), 64);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_drag_packets_vs_block_hopper_tick() {
    let fixture = DeathTestWorld::new().await;
    let chest = chest(&fixture, 4);
    let world = fixture.world();
    let hopper_position = chest.position.down();
    world.set_block_state(
        &hopper_position,
        Block::HOPPER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let hopper = Arc::new(HopperBlockEntity::new(hopper_position, FacingHopper::Down));
    world.add_block_entity(hopper.clone());
    let player = TestPlayer::new(&world);
    open_chest(&player, Arc::new(PausedInventory(chest.clone())));
    let screen = player.player.current_screen_handler.lock().unwrap().clone();
    *screen
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap() = ItemStack::new(4, &Item::DIAMOND);
    for (slot, button) in [(-999, 0), (0, 1), (1, 1)] {
        let mut packet = click();
        packet.slot = slot;
        packet.button = button;
        packet.mode = SlotActionType::QuickCraft;
        player.player.on_slot_click(packet, &fixture.server);
    }
    let (ready, wait_ready) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _runtime = runtime.enter();
            READ_PAUSE.with(|slot| {
                *slot.borrow_mut() = Some((
                    0,
                    2,
                    Box::new(move || {
                        ready.send(()).unwrap();
                        wait_resume.recv().unwrap();
                    }),
                ));
            });
            let mut packet = click();
            packet.slot = -999;
            packet.button = 2;
            packet.mode = SlotActionType::QuickCraft;
            player.player.on_slot_click(packet, &fixture.server);
        });
        wait_ready.recv().unwrap();
        hopper.tick(&world);
        resume.send(()).unwrap();
    });
    assert_eq!(
        (
            diamonds(chest.as_ref()),
            diamonds(hopper.as_ref()),
            cursor(&player)
        ),
        (7, 1, 0)
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_save_all_and_load_during_ender_chest_transfer() {
    let fixture = DeathTestWorld::new().await;
    let player = TestPlayer::new(&fixture.world());
    let storage = &fixture.server.player_data_storage;
    let (session, _) = storage
        .load_data(&player.player.gameprofile.id)
        .await
        .unwrap();
    *player.player.storage_session.lock().unwrap() = Some(session);
    player
        .player
        .ender_chest_inventory
        .set_stack(0, ItemStack::new(64, &Item::DIAMOND));
    open_chest(
        &player,
        Arc::new(PausedInventory(player.player.ender_chest_inventory.clone())),
    );
    let (withdrawn, wait_withdrawn) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    let (saving, wait_saving) = mpsc::channel();
    let (save_resume, wait_save_resume) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _runtime = runtime.enter();
            WRITE_PAUSE.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    withdrawn.send(()).unwrap();
                    wait_resume.recv().unwrap();
                }));
            });
            player.player.on_slot_click(click(), &fixture.server);
        });
        wait_withdrawn.recv().unwrap();
        crate::entity::player_inventory_save::set_save_pause(
            player.player.gameprofile.id,
            Box::new(move || {
                saving.send(()).unwrap();
                wait_save_resume.recv().unwrap();
            }),
        );
        let source = CommandSender::Console.into_source(&fixture.server);
        assert_eq!(
            fixture
                .server
                .command_dispatcher
                .load()
                .execute_input("save-all", &source),
            Ok(1)
        );
        wait_saving.recv().unwrap();
        assert!(
            player
                .player
                .inventory
                .slot_transaction(0)
                .try_lock()
                .is_none()
        );
        resume.send(()).unwrap();
        save_resume.send(()).unwrap();
    });
    crate::server::combat_test_support::wait_for_tasks(&fixture.server).await;
    player.player.storage_session.lock().unwrap().take();
    let (session, saved) = storage
        .load_data(&player.player.gameprofile.id)
        .await
        .unwrap();
    let loaded = TestPlayer::new(&fixture.world());
    EntityBase::read_nbt_non_mut(loaded.player.as_ref(), &saved.unwrap());
    assert_eq!(
        diamonds(loaded.player.inventory.as_ref())
            + diamonds(loaded.player.ender_chest_inventory.as_ref()),
        64
    );
    drop(session);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_break_chest_during_failed_cart_transfer() {
    let fixture = DeathTestWorld::new().await;
    let chest = chest(&fixture, 64);
    let world = fixture.world();
    world.add_block_entity(Arc::new(PausedChest {
        chest: chest.clone(),
        inventory: Arc::new(PausedInventory(chest.clone())),
    }));
    let cart = Arc::new(MinecartEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::HOPPER_MINECART,
    )));
    let inventory = cart.container().unwrap();
    for slot in 0..inventory.size() {
        inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    let (withdrawn, wait_withdrawn) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    let (breaking, wait_breaking) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _runtime = runtime.enter();
            WRITE_PAUSE.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    withdrawn.send(()).unwrap();
                    wait_resume.recv().unwrap();
                }));
            });
            cart.tick(cart.as_ref(), &fixture.server);
        });
        wait_withdrawn.recv().unwrap();
        scope.spawn(|| {
            let _runtime = runtime.enter();
            RETIRE_PAUSE.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    breaking.send(()).unwrap();
                }));
            });
            world.break_block(&chest.position, None, BlockFlags::NOTIFY_ALL);
        });
        wait_breaking.recv().unwrap();
        resume.send(()).unwrap();
    });
    assert_eq!(world.get_block(&chest.position), &Block::AIR);
    assert!(chest.is_empty());
    let drops: u32 = world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| {
            let stack = item.get_item_stack().lock().unwrap();
            if stack.item == &Item::DIAMOND {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum();
    assert_eq!(drops + diamonds(inventory.as_ref()), 64);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_hopper_tick_clears_jukebox_has_record() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let position = BlockPos::new(8, 65, 8);
    world.set_block_state(
        &position,
        Block::JUKEBOX.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let jukebox = Arc::new(JukeboxBlockEntity::new(position));
    world.add_block_entity(jukebox.clone());
    jukebox.set_record(ItemStack::new(1, &Item::MUSIC_DISC_13));
    world.set_block_state(
        &position.down(),
        Block::HOPPER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let hopper = Arc::new(HopperBlockEntity::new(position.down(), FacingHopper::Down));
    world.add_block_entity(hopper.clone());
    hopper.tick(&world);
    assert!(jukebox.is_empty());
    assert_eq!(hopper.get_stack(0).item, &Item::MUSIC_DISC_13);
    assert!(!JukeboxLikeProperties::from_state_id(world.get_block_state_id(&position)).has_record);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_furnace_counts_only_admitted_output() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let position = BlockPos::new(8, 65, 8);
    world.set_block_state(
        &position,
        Block::FURNACE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let furnace = Arc::new(crate::block::entities::furnace::FurnaceBlockEntity::new(
        position,
    ));
    furnace.set_stack(2, ItemStack::new(64, &Item::IRON_INGOT));
    world.add_block_entity(furnace.clone());
    world.set_block_state(
        &position.down(),
        Block::HOPPER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let hopper = Arc::new(HopperBlockEntity::new(position.down(), FacingHopper::Down));
    world.add_block_entity(hopper.clone());
    let player = TestPlayer::new(&world);
    let properties: Arc<dyn pumpkin_inventory::window_property::PropertyDelegate> = furnace.clone();
    let handler =
        pumpkin_inventory::furnace_like::furnace_like_screen_handler::FurnaceLikeScreenHandler::new(
            1,
            &player.player.inventory,
            Arc::new(PausedInventory(furnace.clone())),
            &properties,
            furnace.clone(),
            pumpkin_data::screen::WindowType::Furnace,
        );
    *player.player.current_screen_handler.lock().unwrap() =
        Arc::new(std::sync::Mutex::new(handler));
    let (ready, wait_ready) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _runtime = runtime.enter();
            READ_PAUSE.with(|slot| {
                *slot.borrow_mut() = Some((
                    2,
                    3,
                    Box::new(move || {
                        ready.send(()).unwrap();
                        wait_resume.recv().unwrap();
                    }),
                ));
            });
            let mut packet = click();
            packet.slot = 2;
            player.player.on_slot_click(packet, &fixture.server);
        });
        wait_ready.recv().unwrap();
        hopper.tick(&world);
        resume.send(()).unwrap();
    });
    let received: u32 = (0..player.player.inventory.size())
        .map(|slot| player.player.inventory.get_stack(slot))
        .filter(|stack| stack.item == &Item::IRON_INGOT)
        .map(|stack| u32::from(stack.item_count))
        .sum();
    assert_eq!((received, hopper.get_stack(0).item_count), (63, 1));
    assert!(furnace.is_empty());
    // AbstractFurnaceMenu.quickMoveStack / FurnaceResultSlot.checkTakeAchievements.
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Crafted,
            i32::from(Item::IRON_INGOT.id),
        ),
        63
    );
    fixture.server.shutdown().await;
}
