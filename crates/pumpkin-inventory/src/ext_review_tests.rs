//! External review reproductions using production menus and slots.
use crate::{
    Inventory, SimpleInventory,
    anvil::anvil_screen_handler::AnvilScreenHandler,
    crafting::crafting_screen_handler::CraftingTableScreenHandler,
    entity_equipment::EntityEquipment,
    player::player_inventory::PlayerInventory,
    screen_handler::{InventoryPlayer, ScreenHandler},
};
use pumpkin_data::{
    data_component_impl::{BundleContentsImpl, EquipmentSlot},
    item::Item,
    item_stack::ItemStack,
    screen::WindowType,
    sound::Sound,
    statistic::StatisticCategory,
};
use pumpkin_protocol::java::{
    client::play::{
        CSetContainerContent, CSetContainerProperty, CSetContainerSlot, CSetCursorItem,
        CSetPlayerInventory, CSetSelectedSlot,
    },
    server::play::SlotActionType,
};
use std::{
    any::Any,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, Ordering::Relaxed},
    },
};

struct Player {
    inventory: Arc<PlayerInventory>,
    drops: Mutex<Vec<ItemStack>>,
    drop_limit: usize,
    levels: AtomicI32,
}

impl Player {
    fn new(drop_limit: usize) -> Self {
        Self {
            inventory: Arc::new(PlayerInventory::new(
                Arc::new(Mutex::new(EntityEquipment::new())),
                Arc::new(crate::build_equipment_slots()),
            )),
            drops: Mutex::default(),
            drop_limit,
            levels: AtomicI32::new(10),
        }
    }

    fn fill_inventory_except_one(&self, stack: ItemStack) {
        for index in 0..PlayerInventory::MAIN_SIZE {
            self.inventory
                .set_stack(index, ItemStack::new(64, &Item::DIRT));
        }
        self.inventory.set_stack(0, stack);
    }
}

impl InventoryPlayer for Player {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn drop_item(&self, stack: ItemStack, _retain_ownership: bool) {
        let mut drops = self.drops.lock().unwrap();
        drops.push(stack);
        // Bound a broken synchronous THROW loop without leaving a spinning worker behind.
        assert!(
            drops.len() <= self.drop_limit,
            "THROW exceeded {} drops: {} dropped",
            self.drop_limit,
            drops.len()
        );
    }
    fn get_inventory(&self) -> Arc<PlayerInventory> {
        self.inventory.clone()
    }
    fn has_infinite_materials(&self) -> bool {
        false
    }
    fn is_creative(&self) -> bool {
        false
    }
    fn experience_level(&self) -> i32 {
        self.levels.load(Relaxed)
    }
    fn add_experience_levels(&self, levels: i32) {
        self.levels.fetch_add(levels, Relaxed);
    }
    fn enchantment_seed(&self) -> i32 {
        0
    }
    fn set_enchantment_seed(&self, _seed: i32) {}
    fn enqueue_inventory_packet(
        &self,
        _packet: &CSetContainerContent,
        _window_type: Option<WindowType>,
    ) {
    }
    fn enqueue_slot_packet(
        &self,
        _packet: &CSetContainerSlot,
        _window_type: Option<WindowType>,
        _total_slots: usize,
    ) {
    }
    fn enqueue_cursor_packet(&self, _packet: &CSetCursorItem) {}
    fn enqueue_property_packet(&self, _packet: &CSetContainerProperty) {}
    fn enqueue_slot_set_packet(&self, _packet: &CSetPlayerInventory) {}
    fn enqueue_set_held_item_packet(&self, _packet: &CSetSelectedSlot) {}
    fn enqueue_equipment_change(&self, _slot: &EquipmentSlot, _stack: &ItemStack) {}
    fn award_experience(&self, _amount: i32) {}
    fn increment_stat(&self, _category: StatisticCategory, _stat_id: i32, _amount: i32) {}
    fn play_block_sound(&self, _sound: Sound, _pitch: f32) {}
}

fn diamond_recipe(player: &Player, blocks: u8) -> CraftingTableScreenHandler {
    let handler = CraftingTableScreenHandler::new(1, &player.inventory, None);
    handler.get_behaviour().slots[1].set_stack(ItemStack::new(blocks, &Item::DIAMOND_BLOCK));
    handler.get_behaviour().slots[0].set_stack(ItemStack::EMPTY.clone());
    assert_eq!(
        handler.get_behaviour().slots[0].get_stack().item,
        &Item::DIAMOND
    );
    handler
}

#[ignore = "external-review reproduction R03; passes once survival task 1 lands"]
#[test]
fn ext_review_r03_ctrl_q_crafting_result_terminates_after_one_craft() {
    // AbstractContainerMenu.doClick: stop when safeTake returns empty or output changes.
    let player = Player::new(1);
    let mut handler = diamond_recipe(&player, 1);
    handler.on_slot_click(0, 1, SlotActionType::Throw, &player);
    let drops = player.drops.lock().unwrap();
    assert_eq!(drops.len(), 1);
    assert_eq!(drops[0].item_count, 9);
    assert!(handler.get_behaviour().slots[1].get_stack().is_empty());
    assert!(handler.get_behaviour().slots[0].get_stack().is_empty());
}

#[ignore = "external-review reproduction R03; passes once survival task 1 lands"]
#[test]
fn ext_review_r03_single_q_charges_ingredients_once() {
    // AbstractContainerMenu.doClick calls Slot.safeTake, which already calls ResultSlot.onTake.
    let player = Player::new(1);
    let mut handler = diamond_recipe(&player, 2);
    handler.on_slot_click(0, 0, SlotActionType::Throw, &player);
    let drops = player.drops.lock().unwrap();
    assert_eq!(drops.len(), 1);
    assert_eq!(
        (
            handler.get_behaviour().slots[1].get_stack().item_count,
            drops[0].item_count
        ),
        (1, 1)
    );
}

#[ignore = "external-review reproduction R04; passes once survival task 1 lands"]
#[test]
fn ext_review_r04_cursor_bundle_consumes_crafting_ingredients() {
    // BundleItem.overrideStackedOnOther -> BundleContents.Mutable.tryTransfer -> Slot.safeTake.
    let player = Player::new(1);
    let mut handler = diamond_recipe(&player, 1);
    let mut bundle = ItemStack::new(1, &Item::BUNDLE);
    bundle.set_data_component(BundleContentsImpl { items: Vec::new() });
    *handler.get_behaviour().cursor_stack.lock().unwrap() = bundle;
    handler.on_slot_click(0, 0, SlotActionType::Pickup, &player);
    let cursor = handler.get_behaviour().cursor_stack.lock().unwrap();
    let contents = cursor.get_data_component::<BundleContentsImpl>().unwrap();
    assert_eq!(
        contents
            .items
            .iter()
            .map(|stack| u32::from(stack.item_count))
            .sum::<u32>(),
        9
    );
    assert!(
        handler.get_behaviour().slots[1].get_stack().is_empty(),
        "bundle received output without paying its ingredient"
    );
    assert!(handler.get_behaviour().slots[0].get_stack().is_empty());
}

#[ignore = "external-review reproduction R04; passes once survival task 1 lands"]
#[test]
fn ext_review_r04_secondary_bundle_click_cannot_duplicate_result() {
    // BundleItem.overrideStackedOnOther handles SECONDARY only when the other slot is empty.
    let player = Player::new(1);
    let mut handler = diamond_recipe(&player, 1);
    let mut bundle = ItemStack::new(1, &Item::BUNDLE);
    bundle.set_data_component(BundleContentsImpl { items: Vec::new() });
    *handler.get_behaviour().cursor_stack.lock().unwrap() = bundle;
    for _ in 0..8 {
        handler.on_slot_click(0, 1, SlotActionType::Pickup, &player);
    }
    let cursor = handler.get_behaviour().cursor_stack.lock().unwrap();
    let contents = cursor.get_data_component::<BundleContentsImpl>().unwrap();
    assert_eq!(
        contents
            .items
            .iter()
            .map(|stack| u32::from(stack.item_count))
            .sum::<u32>(),
        0
    );
    assert_eq!(handler.get_behaviour().slots[1].get_stack().item_count, 1);
}

#[ignore = "external-review reproduction R05; passes once survival task 1 lands"]
#[test]
fn ext_review_r05_partial_shift_click_persists_crafting_grid_remainder() {
    // CraftingMenu.quickMoveStack mutates the real source stack before Slot.setChanged.
    let player = Player::new(1);
    player.fill_inventory_except_one(ItemStack::new(63, &Item::COBBLESTONE));
    let mut handler = CraftingTableScreenHandler::new(1, &player.inventory, None);
    handler.get_behaviour().slots[1].set_stack(ItemStack::new(64, &Item::COBBLESTONE));
    handler.on_slot_click(1, 0, SlotActionType::QuickMove, &player);
    assert_eq!(player.inventory.get_stack(0).item_count, 64);
    assert_eq!(handler.get_behaviour().slots[1].get_stack().item_count, 63);
}

fn renamed_diamonds(player: &Player) -> AnvilScreenHandler {
    let inventory = Arc::new(SimpleInventory::new(3));
    inventory.set_stack(0, ItemStack::new(64, &Item::DIAMOND));
    let mut handler = AnvilScreenHandler::new(1, &player.inventory, inventory);
    assert!(handler.set_item_name("review", false));
    assert_eq!(handler.inventory.get_stack(2).item_count, 64);
    handler
}

#[test]
fn ext_review_r06_anvil_cursor_rejects_partial_capacity() {
    // Slot.tryRemove disallows partial result pickup when maxAmount < result count.
    let player = Player::new(1);
    let mut handler = renamed_diamonds(&player);
    let carried = handler.inventory.get_stack(2).copy_with_count(63);
    *handler.get_behaviour().cursor_stack.lock().unwrap() = carried;
    handler.on_slot_click(2, 0, SlotActionType::Pickup, &player);
    assert_eq!(handler.inventory.get_stack(0).item_count, 64);
    assert_eq!(handler.inventory.get_stack(2).item_count, 64);
    assert_eq!(
        handler
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .item_count,
        63
    );
    assert_eq!(player.experience_level(), 10);
}

#[test]
fn ext_review_r06_anvil_partial_shift_click_matches_vanilla_on_take() {
    // ItemCombinerMenu.quickMoveStack permits a partial move; AnvilMenu.onTake clears input.
    // Its inputSlots.setChanged -> slotsChanged -> createResult also clears the remaining output.
    let player = Player::new(1);
    let mut handler = renamed_diamonds(&player);
    player.fill_inventory_except_one(handler.inventory.get_stack(2).copy_with_count(63));
    handler.on_slot_click(2, 0, SlotActionType::QuickMove, &player);
    assert_eq!(player.inventory.get_stack(0).item_count, 64);
    assert!(handler.inventory.get_stack(0).is_empty());
    assert!(handler.inventory.get_stack(2).is_empty());
    assert_eq!(player.experience_level(), 9);
    assert!(player.drops.lock().unwrap().is_empty());
}
