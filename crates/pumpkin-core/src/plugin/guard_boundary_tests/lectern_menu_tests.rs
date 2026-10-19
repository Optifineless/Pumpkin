use super::{Fixture, assert_menu_unlocked};
use crate::{
    block::{
        BlockBehaviour, GetScreenHandlerFactoryArgs,
        blocks::lectern::LecternBlock,
        entities::{BlockEntity, lectern::LecternBlockEntity},
    },
    world::guard_effects::GuardEffects,
};
use pumpkin_data::{
    Block, BlockStateId, block_properties::LecternLikeProperties,
    data_component_impl::WritableBookContentImpl, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SContainerButtonClick};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering};

const POSITION: BlockPos = BlockPos::new(2, 64, 2);

fn book(label: &str) -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::WRITABLE_BOOK);
    stack.set_data_component(WritableBookContentImpl {
        pages: (0..3).map(|page| format!("{label} page {page}")).collect(),
    });
    stack
}

fn place_lectern(fixture: &Fixture, book: &ItemStack, page: usize) -> Arc<LecternBlockEntity> {
    let mut properties = LecternLikeProperties::default(&Block::LECTERN);
    properties.has_book = true;
    fixture.world.set_block_state(
        &POSITION,
        properties.to_state_id(&Block::LECTERN),
        BlockFlags::NOTIFY_ALL,
    );
    let entity = Arc::new(LecternBlockEntity::new(POSITION));
    entity.set_stack(0, book.clone());
    entity.page.store(page, Ordering::Relaxed);
    fixture.world.add_block_entity(entity.clone());
    let live: Arc<dyn BlockEntity> = entity.clone();
    assert!(fixture.world.is_live_block_entity(&live));
    entity
}

fn open_lectern(fixture: &Fixture) -> i32 {
    let player = &fixture.player.player;
    let factory = LecternBlock
        .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: &fixture.server,
            world: &fixture.world,
            block: &Block::LECTERN,
            position: &POSITION,
            player,
        })
        .unwrap();
    player
        .open_handled_screen(factory.as_ref(), Some(POSITION))
        .unwrap();
    let menu = player.current_screen_handler.lock().unwrap().clone();
    let sync_id = menu.lock().unwrap().sync_id();
    i32::from(sync_id)
}

fn click(fixture: &Fixture, window_id: i32, button: i32) {
    fixture
        .player
        .player
        .on_container_button_click(&SContainerButtonClick {
            window_id: VarInt(window_id),
            button_id: VarInt(button),
        });
    assert_menu_unlocked(&fixture.player.player);
}

fn properties(fixture: &Fixture) -> LecternLikeProperties {
    LecternLikeProperties::from_state_id(fixture.world.get_block_state(&POSITION).id)
}

fn book_counts(fixture: &Fixture, book: &ItemStack) -> (u32, u32) {
    let inventory = &fixture.player.player.inventory;
    let held = (0..inventory.size())
        .map(|index| inventory.get_stack(index))
        .filter(|stack| stack.are_items_and_components_equal(book))
        .map(|stack| u32::from(stack.item_count))
        .sum();
    let dropped = fixture
        .world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| entity.get_item_stack().lock().unwrap().clone())
        .filter(|stack| stack.are_items_and_components_equal(book))
        .map(|stack| u32::from(stack.item_count))
        .sum();
    (held, dropped)
}

async fn close(fixture: Fixture) {
    tokio::time::timeout(std::time::Duration::from_secs(5), fixture.close())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_page_button_updates_page_and_power_after_unlocking() {
    let fixture = Fixture::new();
    let book = book("original");
    let lectern = place_lectern(&fixture, &book, 0);
    let window = open_lectern(&fixture);

    click(&fixture, window, 2);

    assert_eq!(lectern.page.load(Ordering::Relaxed), 1);
    assert!(properties(&fixture).powered);
    assert!(properties(&fixture).has_book);
    assert!(lectern.get_stack(0).are_equal(&book));
    assert_eq!(book_counts(&fixture, &book), (0, 0));
    close(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_take_button_delivers_book_once_and_clears_state() {
    let fixture = Fixture::new();
    let book = book("original");
    let lectern = place_lectern(&fixture, &book, 2);
    let window = open_lectern(&fixture);
    let mut state = properties(&fixture);
    state.powered = true;
    fixture.world.set_block_state(
        &POSITION,
        state.to_state_id(&Block::LECTERN),
        BlockFlags::NOTIFY_ALL,
    );

    click(&fixture, window, 3);
    click(&fixture, window, 3);

    assert!(lectern.get_stack(0).is_empty());
    assert_eq!(lectern.page.load(Ordering::Relaxed), 0);
    assert!(!properties(&fixture).has_book);
    assert!(!properties(&fixture).powered);
    assert_eq!(book_counts(&fixture, &book), (1, 0));
    close(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_take_button_drops_book_once_when_inventory_is_full() {
    let fixture = Fixture::new();
    let book = book("original");
    let lectern = place_lectern(&fixture, &book, 1);
    for index in 0..36 {
        fixture
            .player
            .player
            .inventory
            .set_stack(index, ItemStack::new(64, &Item::STONE));
    }
    let window = open_lectern(&fixture);

    click(&fixture, window, 3);
    click(&fixture, window, 3);

    assert!(lectern.get_stack(0).is_empty());
    assert!(!properties(&fixture).has_book);
    assert_eq!(book_counts(&fixture, &book), (0, 1));
    close(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_queued_page_button_preserves_replacement() {
    let fixture = Fixture::new();
    let original = book("original");
    let lectern = place_lectern(&fixture, &original, 0);
    let window = open_lectern(&fixture);
    let replacement_book = book("replacement");
    let effects = GuardEffects::new();

    click(&fixture, window, 2);
    let replacement = place_lectern(&fixture, &replacement_book, 2);
    drop(effects);

    assert_eq!(lectern.page.load(Ordering::Relaxed), 0);
    assert_eq!(replacement.page.load(Ordering::Relaxed), 2);
    assert!(replacement.get_stack(0).are_equal(&replacement_book));
    assert!(properties(&fixture).has_book);
    assert!(!properties(&fixture).powered);
    assert_eq!(book_counts(&fixture, &original), (0, 0));
    close(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_queued_take_button_delivers_once_and_preserves_replacement() {
    let fixture = Fixture::new();
    let original = book("original");
    let lectern = place_lectern(&fixture, &original, 2);
    let window = open_lectern(&fixture);
    let replacement_book = book("replacement");
    let effects = GuardEffects::new();

    click(&fixture, window, 3);
    click(&fixture, window, 3);
    let replacement = place_lectern(&fixture, &replacement_book, 1);
    drop(effects);

    assert!(lectern.get_stack(0).is_empty());
    assert_eq!(replacement.page.load(Ordering::Relaxed), 1);
    assert!(replacement.get_stack(0).are_equal(&replacement_book));
    assert!(properties(&fixture).has_book);
    assert!(!properties(&fixture).powered);
    assert_eq!(book_counts(&fixture, &original), (1, 0));
    assert_eq!(book_counts(&fixture, &replacement_book), (0, 0));
    close(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_lectern_queued_take_button_drops_once_after_lectern_vanishes() {
    let fixture = Fixture::new();
    let original = book("original");
    let lectern = place_lectern(&fixture, &original, 2);
    for index in 0..36 {
        fixture
            .player
            .player
            .inventory
            .set_stack(index, ItemStack::new(64, &Item::STONE));
    }
    let window = open_lectern(&fixture);
    let effects = GuardEffects::new();

    click(&fixture, window, 3);
    fixture
        .world
        .set_block_state(&POSITION, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
    click(&fixture, window, 3);
    drop(effects);

    assert!(lectern.get_stack(0).is_empty());
    assert_eq!(
        fixture.world.get_block_state(&POSITION).id,
        BlockStateId::AIR
    );
    assert!(fixture.world.get_block_entity(&POSITION).is_none());
    assert_eq!(book_counts(&fixture, &original), (0, 1));
    close(fixture).await;
}
