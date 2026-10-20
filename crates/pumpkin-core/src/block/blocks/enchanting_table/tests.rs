use super::*;
use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
use pumpkin_data::{
    Block, data_component_impl::StoredEnchantmentsImpl, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::{
    Inventory, enchanting::enchanting_screen_handler::EnchantingTableScreenHandler,
    screen_handler::SharedScreenHandler,
};
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SContainerButtonClick, SlotActionType},
};
use pumpkin_util::{
    PermissionLvl,
    math::{position::BlockPos, vector2::Vector2},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering};

const TABLE_POS: BlockPos = BlockPos::new(8, 64, 8);

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<crate::server::Server>,
    world: Arc<crate::world::World>,
    player: TestPlayer,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let player = TestPlayer::new(&world);
        player.player.permission_lvl.store(PermissionLvl::Four);
        player.client().set_player(player.player.clone());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        Self {
            _directory: directory,
            server,
            world,
            player,
        }
    }

    fn set_block(&self, pos: BlockPos, block: &'static Block) {
        self.world
            .set_block_state(&pos, block.default_state.id, BlockFlags::FORCE_STATE);
    }

    fn open_table(&self, pos: BlockPos) -> (u8, SharedScreenHandler) {
        let factory = EnchantingTableBlock
            .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
                server: &self.server,
                world: &self.world,
                block: &Block::ENCHANTING_TABLE,
                position: &pos,
                player: &self.player.player,
            })
            .unwrap();
        let sync_id = self
            .player
            .player
            .open_handled_screen(factory.as_ref(), Some(pos))
            .unwrap();
        let handler = self
            .player
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone();
        (sync_id, handler)
    }

    fn click_slot(&self, sync_id: u8, slot: i16, mode: SlotActionType) {
        self.player.player.on_slot_click(
            SClickSlot {
                sync_id: VarInt(i32::from(sync_id)),
                revision: VarInt(0),
                slot,
                button: 0,
                mode,
                length_of_array: VarInt(0),
                array_of_changed_slots: Vec::new(),
                carried_item: OptionalItemStackHash(None),
            },
            &self.server,
        );
    }

    fn move_inventory_stack_to_table(&self, sync_id: u8, inventory_slot: usize) {
        let menu_slot = if inventory_slot < 9 {
            inventory_slot + 29
        } else {
            inventory_slot - 7
        };
        self.click_slot(sync_id, menu_slot as i16, SlotActionType::QuickMove);
    }

    async fn close(self) {
        self.player.client().close();
        self.world.level.shutdown().await.unwrap();
    }
}

#[path = "b1020b_g02_tests.rs"]
mod b1020b_g02_tests;

fn table_values(menu: &SharedScreenHandler) -> ([i32; 3], [i32; 3], [i32; 3], i32) {
    let guard = menu.lock().unwrap();
    let table = guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap();
    (
        table.level_requirements,
        table.enchantment_id,
        table.enchantment_level,
        table.bookshelf_count,
    )
}

fn add_fifteen_bookshelves(fixture: &Fixture, pos: BlockPos) {
    let mut added = 0;
    for y in 0..=1 {
        for z in [-2, 0, 2] {
            for x in [-2, 0, 2] {
                if x == 0 && z == 0 {
                    continue;
                }
                fixture.set_block(pos.add(x, y, z), &Block::BOOKSHELF);
                added += 1;
                if added == 15 {
                    return;
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enchanting_table_book_offers_convert_to_one_stored_enchantment() {
    let fixture = Fixture::new();
    fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    add_fifteen_bookshelves(&fixture, TABLE_POS);
    fixture
        .player
        .player
        .enchantment_seed
        .store(0, Ordering::Relaxed);
    fixture
        .player
        .player
        .experience_level
        .store(30, Ordering::Relaxed);
    let (sync_id, menu) = fixture.open_table(TABLE_POS);

    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture
        .player
        .player
        .inventory
        .set_stack(1, ItemStack::new(3, &Item::LAPIS_LAZULI));
    fixture.move_inventory_stack_to_table(sync_id, 0);
    fixture.move_inventory_stack_to_table(sync_id, 1);

    let (costs, equipment_ids, _, bookshelves) = table_values(&menu);
    assert_eq!(bookshelves, 15);
    assert!(costs[2] > 0);
    assert!(equipment_ids.iter().any(|id| *id >= 0));

    // A supported equipment item remains eligible through the same production slot-update path.
    fixture.click_slot(sync_id, 0, SlotActionType::QuickMove);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::BOOK));
    fixture.move_inventory_stack_to_table(sync_id, 0);
    let (book_costs, book_ids, book_levels, _) = table_values(&menu);
    assert!(book_costs.iter().any(|cost| *cost > 0));
    assert!(book_ids[2] >= 0);

    // With 15 shelves and this fixed seed, offer 2 rolls multiple compatible results before
    // the vanilla regular-book rule removes one of them.
    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(i32::from(sync_id)),
            button_id: VarInt(2),
        });

    let guard = menu.lock().unwrap();
    let table = guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap();
    let enchanted_book = table.inventory.get_stack(0);
    assert_eq!(enchanted_book.item, &Item::ENCHANTED_BOOK);
    let stored = enchanted_book
        .get_data_component::<StoredEnchantmentsImpl>()
        .unwrap();
    assert_eq!(stored.enchantment.len(), 1);
    assert_eq!(stored.enchantment[0].0.id as i32, book_ids[2]);
    assert_eq!(stored.enchantment[0].1, book_levels[2]);
    assert!(
        enchanted_book
            .get_data_component::<pumpkin_data::data_component_impl::EnchantmentsImpl>()
            .is_none_or(|regular| regular.enchantment.is_empty())
    );
    drop(guard);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enchanting_table_book_stack_enchants_one_book() {
    let fixture = Fixture::new();
    fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    add_fifteen_bookshelves(&fixture, TABLE_POS);
    fixture
        .player
        .player
        .enchantment_seed
        .store(0, Ordering::Relaxed);
    fixture
        .player
        .player
        .experience_level
        .store(30, Ordering::Relaxed);
    let (sync_id, menu) = fixture.open_table(TABLE_POS);

    let inventory = &fixture.player.player.inventory;
    inventory.set_stack(0, ItemStack::new(64, &Item::BOOK));
    inventory.set_stack(1, ItemStack::new(3, &Item::LAPIS_LAZULI));
    fixture.move_inventory_stack_to_table(sync_id, 0);
    fixture.move_inventory_stack_to_table(sync_id, 1);

    // EnchantmentMenu's item slot holds one item; the rest of the stack stays with the player.
    let table_inventory = {
        let guard = menu.lock().unwrap();
        guard
            .as_any()
            .downcast_ref::<EnchantingTableScreenHandler>()
            .unwrap()
            .inventory
            .clone()
    };
    assert_eq!(table_inventory.get_stack(0).item_count, 1);
    assert_eq!(inventory.get_stack(0).item, &Item::BOOK);
    assert_eq!(inventory.get_stack(0).item_count, 63);
    let (_, book_ids, _, _) = table_values(&menu);
    assert!(book_ids[2] >= 0);

    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(i32::from(sync_id)),
            button_id: VarInt(2),
        });

    let enchanted = table_inventory.get_stack(0);
    assert_eq!(enchanted.item, &Item::ENCHANTED_BOOK);
    assert_eq!(enchanted.item_count, 1);
    assert_eq!(inventory.get_stack(0).item_count, 63);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enchanting_table_middle_offer_uses_vanilla_requirement_and_option_payment() {
    let fixture = Fixture::new();
    fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    fixture
        .player
        .player
        .enchantment_seed
        .store(0, Ordering::Relaxed);
    let (sync_id, menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture
        .player
        .player
        .inventory
        .set_stack(1, ItemStack::new(2, &Item::LAPIS_LAZULI));
    fixture.move_inventory_stack_to_table(sync_id, 0);
    fixture.move_inventory_stack_to_table(sync_id, 1);
    assert_eq!(table_values(&menu).0, [2, 2, 6]);

    add_fifteen_bookshelves(&fixture, TABLE_POS);
    let (shelf_sync_id, shelf_menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture
        .player
        .player
        .inventory
        .set_stack(1, ItemStack::new(2, &Item::LAPIS_LAZULI));
    fixture.move_inventory_stack_to_table(shelf_sync_id, 0);
    fixture.move_inventory_stack_to_table(shelf_sync_id, 1);
    let (shelf_costs, shelf_ids, _, shelf_count) = table_values(&shelf_menu);
    assert_eq!(shelf_count, 15);
    assert_eq!(shelf_costs, [8, 13, 30]);
    assert!(shelf_ids[1] >= 0);

    fixture
        .player
        .player
        .experience_level
        .store(shelf_costs[1], Ordering::Relaxed);
    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(i32::from(shelf_sync_id)),
            button_id: VarInt(1),
        });

    assert_eq!(
        fixture
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        shelf_costs[1] - 2
    );
    let guard = shelf_menu.lock().unwrap();
    let table = guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap();
    assert!(table.inventory.get_stack(0).has_enchantments());
    assert_eq!(table.inventory.get_stack(1).item_count, 0);
    drop(guard);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enchanting_table_counts_each_bookshelf_transmitter_height_separately() {
    let fixture = Fixture::new();
    fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    let lower_shelf = TABLE_POS.add(2, 0, 0);
    let lower_gap = TABLE_POS.add(1, 0, 0);
    let upper_gap = TABLE_POS.add(1, 1, 0);
    fixture.set_block(lower_shelf, &Block::BOOKSHELF);

    let (clear_sync, clear_menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.move_inventory_stack_to_table(clear_sync, 0);
    let (clear_costs, _, _, clear_count) = table_values(&clear_menu);
    assert_eq!(clear_count, 1);

    fixture.set_block(upper_gap, &Block::STONE);
    let (upper_obstructed_sync, upper_obstructed_menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.move_inventory_stack_to_table(upper_obstructed_sync, 0);
    let (upper_obstructed_costs, _, _, upper_obstructed_count) =
        table_values(&upper_obstructed_menu);
    assert_eq!(upper_obstructed_count, 1);
    assert_eq!(upper_obstructed_costs, clear_costs);

    fixture.set_block(upper_gap, &Block::AIR);
    fixture.set_block(lower_gap, &Block::STONE);
    let (lower_obstructed_sync, lower_obstructed_menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.move_inventory_stack_to_table(lower_obstructed_sync, 0);
    let (_, _, _, lower_obstructed_count) = table_values(&lower_obstructed_menu);
    assert_eq!(lower_obstructed_count, 0);

    fixture.set_block(lower_shelf, &Block::AIR);
    let upper_shelf = TABLE_POS.add(2, 1, 0);
    fixture.set_block(upper_shelf, &Block::BOOKSHELF);
    let (upper_shelf_sync, upper_shelf_menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.move_inventory_stack_to_table(upper_shelf_sync, 0);
    let (upper_shelf_costs, _, _, upper_shelf_count) = table_values(&upper_shelf_menu);
    assert_eq!(upper_shelf_count, 1);
    assert_eq!(upper_shelf_costs, clear_costs);

    fixture.set_block(upper_gap, &Block::STONE);
    let (blocked_upper_sync, blocked_upper_menu) = fixture.open_table(TABLE_POS);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.move_inventory_stack_to_table(blocked_upper_sync, 0);
    let (_, _, _, blocked_upper_count) = table_values(&blocked_upper_menu);
    assert_eq!(blocked_upper_count, 0);

    fixture.close().await;
}
