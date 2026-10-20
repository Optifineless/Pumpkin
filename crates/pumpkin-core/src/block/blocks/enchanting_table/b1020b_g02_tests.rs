use std::sync::atomic::Ordering;

use pumpkin_data::{
    BlockDirection,
    data_component_impl::{EnchantmentsImpl, StoredEnchantmentsImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::{
        client::play::CSetContainerProperty,
        server::play::{SContainerButtonClick, SUseItemOn},
    },
};
use pumpkin_util::{GameMode, math::vector3::Vector3};

use super::*;
use crate::entity::EntityBase;

impl Fixture {
    fn use_table_via_java_packet(&self, pos: BlockPos) -> (u8, SharedScreenHandler) {
        self.player
            .player
            .screen_handler_sync_handler
            .store_player(self.player.player.clone());
        self.player.player.get_entity().set_pos(Vector3::new(
            pos.0.x as f64 + 0.5,
            pos.0.y as f64,
            pos.0.z as f64 + 2.5,
        ));
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: pos,
                    face: VarInt(BlockDirection::Up as i32),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
        let sync_id = self
            .player
            .player
            .screen_handler_sync_id
            .load(Ordering::Relaxed);
        let handler = self
            .player
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone();
        (sync_id, handler)
    }
}

fn property_packet(player: &TestPlayer, sync_id: u8, property: i16, value: i16) -> bytes::Bytes {
    player
        .client()
        .serialize_packet(&CSetContainerProperty::new(
            VarInt(i32::from(sync_id)),
            property,
            value,
        ))
        .unwrap()
}

fn assert_property(
    player: &TestPlayer,
    packets: &[bytes::Bytes],
    sync_id: u8,
    property: i16,
    value: i16,
) {
    let expected = property_packet(player, sync_id, property, value);
    assert!(
        packets.iter().any(|packet| packet == &expected),
        "missing enchantment property {property}={value}; packets={packets:?}"
    );
}

fn click_option(player: &crate::entity::player::Player, sync_id: u8, option: i32) {
    player.on_container_button_click(&SContainerButtonClick {
        window_id: VarInt(i32::from(sync_id)),
        button_id: VarInt(option),
    });
}

fn menu_enchantments(menu: &SharedScreenHandler) -> Vec<(String, i32)> {
    let guard = menu.lock().unwrap();
    let table = guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap();
    table
        .inventory
        .get_stack(0)
        .get_data_component::<EnchantmentsImpl>()
        .map(|enchantments| {
            enchantments
                .enchantment
                .iter()
                .map(|(enchantment, level)| (enchantment.name.to_owned(), *level))
                .collect()
        })
        .unwrap_or_default()
}

fn table_seed(menu: &SharedScreenHandler) -> i32 {
    let guard = menu.lock().unwrap();
    guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap()
        .enchantment_seed
}

fn table_stack(menu: &SharedScreenHandler, slot: usize) -> ItemStack {
    let guard = menu.lock().unwrap();
    guard
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap()
        .inventory
        .get_stack(slot)
}

fn insert_table_inputs(fixture: &Fixture, sync_id: u8, item: ItemStack, lapis: u8) {
    fixture.player.player.inventory.set_stack(0, item);
    if lapis > 0 {
        fixture
            .player
            .player
            .inventory
            .set_stack(1, ItemStack::new(lapis, &Item::LAPIS_LAZULI));
    }
    fixture.move_inventory_stack_to_table(sync_id, 0);
    if lapis > 0 {
        fixture.move_inventory_stack_to_table(sync_id, 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g02_subminimum_offer_is_zero_on_wire() {
    let mut fixture = Fixture::new();
    fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    // Java Random(-122577578) gives the zero-shelf middle offer a raw cost of one.
    let seed = -122_577_578;
    fixture
        .player
        .player
        .enchantment_seed
        .store(seed, Ordering::Relaxed);
    fixture
        .player
        .player
        .experience_level
        .store(30, Ordering::Relaxed);
    let (sync_id, menu) = fixture.use_table_via_java_packet(TABLE_POS);
    assert!(
        !fixture.player.take_packets().is_empty(),
        "use-on should open the table"
    );

    insert_table_inputs(
        &fixture,
        sync_id,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
        2,
    );
    let packets = fixture.player.take_packets();
    let (costs, ids, levels, shelves) = table_values(&menu);
    assert_eq!(shelves, 0);
    assert_property(&fixture.player, &packets, sync_id, 1, 0);
    assert_property(&fixture.player, &packets, sync_id, 5, -1);
    assert_property(&fixture.player, &packets, sync_id, 8, -1);
    assert_eq!(costs, [1, 0, 6]);
    assert_eq!(ids[1], -1);
    assert_eq!(levels[1], -1);

    click_option(&fixture.player.player, sync_id, 1);
    assert_eq!(
        fixture
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        30
    );
    assert_eq!(table_stack(&menu, 1).item_count, 2);
    assert_eq!(table_stack(&menu, 0).item, &Item::DIAMOND_SWORD);
    assert!(fixture.player.player.inventory.get_stack(0).is_empty());
    assert!(menu_enchantments(&menu).is_empty());
    assert_eq!(
        fixture
            .player
            .player
            .enchantment_seed
            .load(Ordering::Relaxed),
        seed
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g02_extra_enchantment_roll_matches_java() {
    verify_extra_roll_witness().await;
}

async fn verify_extra_roll_witness() {
    // The literal expected results were prepared with a separate Java 26.3 oracle for
    // EnchantmentHelper.selectEnchantment, using the supplied RandomSource call order.
    // The negative seed also checks Java's signed int-to-long seed conversion. The Java oracle
    // produced adjusted cost 31 and first extra roll 25; the old threshold was only 16.
    let mut fixture = Fixture::new();
    fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    add_fifteen_bookshelves(&fixture, TABLE_POS);
    let seed = -1_320_574_147;
    fixture
        .player
        .player
        .enchantment_seed
        .store(seed, Ordering::Relaxed);
    fixture
        .player
        .player
        .experience_level
        .store(50, Ordering::Relaxed);
    let (sync_id, menu) = fixture.use_table_via_java_packet(TABLE_POS);
    fixture.player.take_packets();
    insert_table_inputs(
        &fixture,
        sync_id,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
        3,
    );
    let generated = fixture.player.take_packets();
    let (costs, _, _, shelves) = table_values(&menu);
    assert_eq!(table_seed(&menu), seed);
    assert_eq!(shelves, 15);
    assert_eq!(costs, [9, 13, 30]);
    assert_property(&fixture.player, &generated, sync_id, 2, costs[2] as i16);

    click_option(&fixture.player.player, sync_id, 2);
    let actual = menu_enchantments(&menu);
    let damage_enchantments = actual
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "minecraft:sharpness" | "minecraft:smite" | "minecraft:bane_of_arthropods"
            )
        })
        .count();
    assert!(
        damage_enchantments <= 1,
        "incompatible damage enchantments: {actual:?}"
    );
    assert_eq!(
        fixture
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        47
    );
    let table = menu.lock().unwrap();
    let menu_seed = table
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap()
        .enchantment_seed;
    let item = table
        .as_any()
        .downcast_ref::<EnchantingTableScreenHandler>()
        .unwrap()
        .inventory
        .get_stack(0);
    assert_eq!(item.item, &Item::DIAMOND_SWORD);
    assert_eq!(item.item_count, 1);
    let next_seed = fixture
        .player
        .player
        .enchantment_seed
        .load(Ordering::Relaxed);
    assert_ne!(next_seed, seed);
    assert_eq!(menu_seed, next_seed);
    drop(table);
    let (updated_costs, _, _, _) = table_values(&menu);
    assert!(
        updated_costs
            .iter()
            .enumerate()
            .all(|(slot, cost)| *cost == 0 || *cost > slot as i32)
    );
    let after_click = fixture.player.take_packets();
    assert_property(
        &fixture.player,
        &after_click,
        sync_id,
        3,
        next_seed as u16 as i16,
    );
    assert!(table_stack(&menu, 1).is_empty());
    assert_eq!(
        actual,
        vec![
            ("minecraft:bane_of_arthropods".to_owned(), 4),
            ("minecraft:knockback".to_owned(), 2),
        ]
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g02_no_extra_roll_and_creative_payment_control() {
    // Java's first extra roll (45) exceeds the adjusted cost (33), so Sharpness III stands alone.
    let mut control = Fixture::new();
    control.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    add_fifteen_bookshelves(&control, TABLE_POS);
    let no_extra_seed = 678_502_607;
    control
        .player
        .player
        .enchantment_seed
        .store(no_extra_seed, Ordering::Relaxed);
    control.player.player.gamemode.store(GameMode::Creative);
    control
        .player
        .player
        .experience_level
        .store(0, Ordering::Relaxed);
    let (sync_id, menu) = control.use_table_via_java_packet(TABLE_POS);
    control.player.take_packets();
    insert_table_inputs(
        &control,
        sync_id,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
        0,
    );
    assert_eq!(table_values(&menu).0, [8, 11, 30]);
    click_option(&control.player.player, sync_id, 2);
    assert_eq!(
        menu_enchantments(&menu),
        vec![("minecraft:sharpness".to_owned(), 3)]
    );
    assert_eq!(
        control
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        0
    );
    assert!(table_stack(&menu, 1).is_empty());
    let packets = control.player.take_packets();
    let next_seed = control
        .player
        .player
        .enchantment_seed
        .load(Ordering::Relaxed);
    assert_property(
        &control.player,
        &packets,
        sync_id,
        3,
        next_seed as u16 as i16,
    );
    control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g02_valid_offer_and_one_book_limit_remain_passing() {
    let mut fixture = Fixture::new();
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
    let (sync_id, menu) = fixture.use_table_via_java_packet(TABLE_POS);
    fixture.player.take_packets();
    insert_table_inputs(
        &fixture,
        sync_id,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
        2,
    );
    let packets = fixture.player.take_packets();
    let (costs, ids, levels, _) = table_values(&menu);
    assert_eq!(costs, [8, 13, 30]);
    assert!(ids[1] >= 0 && levels[1] > 0);
    assert_property(&fixture.player, &packets, sync_id, 1, 13);
    assert_property(&fixture.player, &packets, sync_id, 5, ids[1] as i16);
    assert_property(&fixture.player, &packets, sync_id, 8, levels[1] as i16);
    click_option(&fixture.player.player, sync_id, 1);
    assert_eq!(
        fixture
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        28
    );
    assert!(table_stack(&menu, 1).is_empty());
    assert!(!menu_enchantments(&menu).is_empty());
    fixture.close().await;

    let mut book_fixture = Fixture::new();
    book_fixture.set_block(TABLE_POS, &Block::ENCHANTING_TABLE);
    add_fifteen_bookshelves(&book_fixture, TABLE_POS);
    book_fixture
        .player
        .player
        .enchantment_seed
        .store(0, Ordering::Relaxed);
    book_fixture
        .player
        .player
        .experience_level
        .store(30, Ordering::Relaxed);
    let (book_sync_id, book_menu) = book_fixture.use_table_via_java_packet(TABLE_POS);
    book_fixture.player.take_packets();
    insert_table_inputs(
        &book_fixture,
        book_sync_id,
        ItemStack::new(64, &Item::BOOK),
        3,
    );
    let book_inventory = {
        let guard = book_menu.lock().unwrap();
        guard
            .as_any()
            .downcast_ref::<EnchantingTableScreenHandler>()
            .unwrap()
            .inventory
            .clone()
    };
    assert_eq!(book_inventory.get_stack(0).item_count, 1);
    assert_eq!(
        book_fixture.player.player.inventory.get_stack(0).item_count,
        63
    );
    click_option(&book_fixture.player.player, book_sync_id, 2);
    let enchanted_book = book_inventory.get_stack(0);
    assert_eq!(enchanted_book.item, &Item::ENCHANTED_BOOK);
    assert_eq!(enchanted_book.item_count, 1);
    assert_eq!(
        book_fixture.player.player.inventory.get_stack(0).item_count,
        63
    );
    let stored = enchanted_book
        .get_data_component::<StoredEnchantmentsImpl>()
        .unwrap();
    assert_eq!(stored.enchantment.len(), 1);
    assert_eq!(
        book_fixture
            .player
            .player
            .experience_level
            .load(Ordering::Relaxed),
        27
    );
    assert_eq!(book_inventory.get_stack(1).item_count, 0);
    book_fixture.close().await;
}
