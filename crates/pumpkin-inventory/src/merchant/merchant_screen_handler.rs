use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use crate::inventory::Inventory;
use pumpkin_data::{
    item_stack::ItemStack,
    screen::WindowType,
    statistic::{CustomStatistic, StatisticCategory},
};

use crate::{
    player::player_inventory::PlayerInventory,
    screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour, offer_or_drop_stack},
    slot::{NormalSlot, Slot},
};

type MerchantValidityCheck = Box<dyn Fn(&dyn InventoryPlayer) -> bool + Send + Sync>;

pub struct MerchantScreenHandler {
    pub inventory: Arc<dyn Inventory>,
    behaviour: ScreenHandlerBehaviour,
    selected_offer: usize,
    active_offer: Option<usize>,
    pub offers: Vec<pumpkin_protocol::java::client::play::MerchantOffer>,
    pub on_trade: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    pub on_trade_updated: Option<Arc<dyn Fn(bool) + Send + Sync>>,
    pub on_close: Option<Arc<dyn Fn() + Send + Sync>>,
    pub validity_check: Option<MerchantValidityCheck>,
    result_taken: Arc<AtomicBool>,
}

impl MerchantScreenHandler {
    pub fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        inventory: Arc<dyn Inventory>,
        offers: Vec<pumpkin_protocol::java::client::play::MerchantOffer>,
    ) -> Self {
        let result_taken = Arc::new(AtomicBool::new(false));
        let mut behaviour = ScreenHandlerBehaviour::new(sync_id, Some(WindowType::Merchant));
        behaviour.container_slots = 3;
        let mut handler = Self {
            inventory,
            behaviour,
            selected_offer: 0,
            active_offer: None,
            offers,
            on_trade: None,
            on_trade_updated: None,
            on_close: None,
            validity_check: None,
            result_taken: result_taken.clone(),
        };

        handler.inventory.on_open();

        for i in 0..2 {
            handler.add_slot(Arc::new(NormalSlot::new(handler.inventory.clone(), i)));
        }
        handler.add_slot(Arc::new(MerchantResultSlot::new(
            handler.inventory.clone(),
            2,
            result_taken,
        )));

        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();
        handler.add_player_slots(&player_inventory);

        handler
    }

    pub fn set_selected_offer(&mut self, index: usize, player: &dyn InventoryPlayer) {
        self.selected_offer = index;
        self.try_move_items(index);
        self.update_result_slot();
        self.notify_trade_updated(player);
        self.send_content_updates();
    }

    fn try_move_items(&self, index: usize) {
        let Some(offer) = self.offers.get(index) else {
            return;
        };
        let cost_a = offer.base_cost_a.0.as_ref().clone();
        let cost_b = offer.cost_b.as_ref().map(|cost| cost.0.as_ref().clone());
        let player_slots_end = self.get_behaviour().slots.len() as i32;

        for index in 0..2 {
            let mut stack = self.inventory.get_stack(index);
            if !stack.is_empty() {
                let mut before = stack.clone();
                let (moved, leftover) = crate::menu_transfer::transfer(
                    &self.get_behaviour().slots,
                    self.get_behaviour().slots[index].as_ref(),
                    &mut stack,
                    (3, player_slots_end, true),
                    &mut before,
                );
                // MerchantMenu's private payment inventory cannot be retired while this menu lives.
                debug_assert!(leftover.is_empty());
                if !moved {
                    return;
                }
            }
        }

        if !self.inventory.get_stack(0).is_empty() || !self.inventory.get_stack(1).is_empty() {
            return;
        }

        self.move_from_inventory_to_payment_slot(0, &cost_a);
        if let Some(cost_b) = cost_b {
            self.move_from_inventory_to_payment_slot(1, &cost_b);
        }
    }

    fn adjusted_cost_a(offer: &pumpkin_protocol::java::client::play::MerchantOffer) -> ItemStack {
        let mut cost = offer.base_cost_a.0.as_ref().clone();
        let demand = i32::from(cost.item_count).saturating_mul(offer.demand) as f32;
        let demand_bonus = (demand * offer.price_multiplier).floor().max(0.0) as i32;
        let count = i32::from(cost.item_count)
            .saturating_add(demand_bonus)
            .saturating_add(offer.special_price)
            .clamp(1, i32::from(cost.get_max_stack_size()));
        cost.set_count(count as u8);
        cost
    }

    fn matches_cost(cost: &ItemStack, input: &ItemStack) -> bool {
        !input.is_empty()
            && cost.are_items_and_components_equal(input)
            && input.item_count >= cost.item_count
    }

    fn satisfies(
        offer: &pumpkin_protocol::java::client::play::MerchantOffer,
        input_a: &ItemStack,
        input_b: &ItemStack,
    ) -> bool {
        Self::matches_cost(&Self::adjusted_cost_a(offer), input_a)
            && offer.cost_b.as_ref().map_or_else(
                || input_b.is_empty(),
                |cost_b| Self::matches_cost(&cost_b.0, input_b),
            )
    }

    fn input_order(
        offer: &pumpkin_protocol::java::client::play::MerchantOffer,
        input_a: &ItemStack,
        input_b: &ItemStack,
    ) -> Option<bool> {
        if Self::satisfies(offer, input_a, input_b) {
            Some(false)
        } else if Self::satisfies(offer, input_b, input_a) {
            Some(true)
        } else {
            None
        }
    }

    fn matching_offer(&self, input_a: &ItemStack, input_b: &ItemStack) -> Option<usize> {
        if self.selected_offer > 0 && self.selected_offer < self.offers.len() {
            return Self::satisfies(&self.offers[self.selected_offer], input_a, input_b)
                .then_some(self.selected_offer);
        }

        self.offers
            .iter()
            .position(|offer| Self::satisfies(offer, input_a, input_b))
    }

    fn find_active_offer(&self, input_a: &ItemStack, input_b: &ItemStack) -> Option<usize> {
        self.matching_offer(input_a, input_b)
            .filter(|&index| !self.offers[index].is_out_of_stock())
            .or_else(|| {
                self.matching_offer(input_b, input_a)
                    .filter(|&index| !self.offers[index].is_out_of_stock())
            })
    }

    // MerchantMenu consumes both costs while the private payment inventory is owned.
    fn commit_payment(&mut self) -> Option<usize> {
        let offer_index = self.active_offer?;
        let offer = self.offers.get(offer_index)?;
        if offer.is_out_of_stock() {
            return None;
        }
        let input_a = self.inventory.get_stack(0);
        let input_b = self.inventory.get_stack(1);
        let swapped = Self::input_order(offer, &input_a, &input_b)?;
        let cost_a = Self::adjusted_cost_a(offer).item_count;
        let cost_b = offer.cost_b.as_ref().map(|cost| cost.0.item_count);
        let inputs = if swapped {
            [&input_b, &input_a]
        } else {
            [&input_a, &input_b]
        };
        let _ = crate::inventory::slot_transaction::withdraw(
            self.inventory.as_ref(),
            usize::from(swapped),
            inputs[0],
            cost_a,
        );
        if let Some(cost_b) = cost_b {
            let _ = crate::inventory::slot_transaction::withdraw(
                self.inventory.as_ref(),
                usize::from(!swapped),
                inputs[1],
                cost_b,
            );
        }
        self.offers[offer_index].uses += 1;
        Some(offer_index)
    }

    fn publish_trade(&self, offer_index: usize, player: &dyn InventoryPlayer) {
        self.inventory.on_slot_changed(0);
        self.inventory.on_slot_changed(1);
        if let Some(on_trade) = &self.on_trade {
            // MerchantResultSlot.onTake: notify the merchant before awarding the trade statistic.
            let callback = on_trade.clone();
            player.run_menu_callback(Box::new(move || callback(offer_index)));
        }
        player.increment_stat(
            StatisticCategory::Custom,
            CustomStatistic::TradedWithVillager as i32,
            1,
        );
    }

    fn complete_trade(&mut self, player: &dyn InventoryPlayer) -> bool {
        let owner = self.inventory.slot_transaction(0);
        let guard = owner.lock();
        if !owner.is_live() {
            return false;
        }
        let committed = self.commit_payment();
        drop(guard);
        committed.is_some_and(|offer_index| {
            self.publish_trade(offer_index, player);
            true
        })
    }

    /// Consumes payment and inserts its result together, then dispatches trade effects.
    pub fn complete_bedrock_trade_into(
        &mut self,
        player: &dyn InventoryPlayer,
        result: &ItemStack,
        destination: Option<&dyn Slot>,
        change: (&ItemStack, ItemStack),
    ) -> bool {
        let destination_inventory = destination.map(Slot::get_inventory);
        if let Some(inventory) = &destination_inventory
            && Arc::ptr_eq(inventory, &self.inventory)
        {
            return false;
        }
        let mut inventories = crate::inventory::slot_transaction::Owners::new();
        self.inventory.collect_transaction_owners(&mut inventories);
        if let Some(inventory) = &destination_inventory {
            inventory.collect_transaction_owners(&mut inventories);
        }
        inventories.sort_unstable_by_key(Arc::as_ptr);
        inventories.dedup_by(|first, second| Arc::ptr_eq(first, second));
        let guards = crate::inventory::slot_transaction::lock_owners(&inventories);
        let output = self.inventory.get_stack(2);
        if !inventories.iter().all(|owner| owner.is_live()) || !output.are_equal(result) {
            return false;
        }
        if let Some(slot) = destination {
            let current = slot.get_stack();
            if current.uid != change.0.uid || !current.are_equal(change.0) {
                return false;
            }
        }
        let Some(offer_index) = self.commit_payment() else {
            return false;
        };
        let _ = crate::inventory::slot_transaction::withdraw(
            self.inventory.as_ref(),
            2,
            &output,
            output.item_count,
        );
        if let Some(slot) = destination {
            let committed = crate::inventory::slot_transaction::exchange(
                slot.get_inventory().as_ref(),
                slot.get_index(),
                change.0,
                change.1,
            );
            debug_assert!(committed);
        }
        drop(guards);
        if let Some(slot) = destination {
            slot.get_inventory().on_slot_changed(slot.get_index());
        }
        self.publish_trade(offer_index, player);
        self.update_result_slot();
        self.notify_trade_updated(player);
        self.send_content_updates();
        true
    }

    /// Completes a trade selected through Bedrock's item-stack request protocol.
    ///
    /// Bedrock moves the result directly between named containers instead of
    /// sending Java's slot-click action, so the caller has already validated
    /// that the result can be inserted into its destination.
    pub fn complete_bedrock_trade(&mut self, player: &dyn InventoryPlayer) -> bool {
        if !self.complete_trade(player) {
            return false;
        }
        self.update_result_slot();
        self.notify_trade_updated(player);
        self.send_content_updates();
        true
    }

    fn can_fully_insert(&self, stack: &ItemStack, start: usize, end: usize) -> bool {
        let mut remaining = stack.item_count;
        for slot in &self.get_behaviour().slots[start..end] {
            if !slot.can_insert(stack) {
                continue;
            }
            let existing = slot.get_cloned_stack();
            let capacity = if existing.is_empty() {
                slot.get_max_item_count_for_stack(stack)
            } else if existing.are_items_and_components_equal(stack)
                && stack.are_items_and_components_equal(&existing)
            {
                slot.get_max_item_count_for_stack(&existing)
                    .saturating_sub(existing.item_count)
            } else {
                0
            };
            remaining = remaining.saturating_sub(capacity);
            if remaining == 0 {
                return true;
            }
        }
        false
    }

    fn move_from_inventory_to_payment_slot(&self, payment_slot: usize, cost: &ItemStack) {
        // MerchantMenu.tryMoveItems transfers actual payment stock, never copied player slots.
        for slot in &self.get_behaviour().slots[3..] {
            let source = slot.get_stack();
            if source.is_empty() || !cost.are_items_and_components_equal(&source) {
                continue;
            }
            let inventory = slot.get_inventory();
            let owners = crate::inventory::slot_transaction::ordered_owners(&[
                inventory.as_ref(),
                self.inventory.as_ref() as &dyn Inventory,
            ]);
            let guards = crate::inventory::slot_transaction::lock_owners(&owners);
            let mut receipt = crate::inventory::slot_transaction::withdraw(
                inventory.as_ref(),
                slot.get_index(),
                &source,
                source.item_count,
            );
            crate::inventory::slot_transaction::deposit(
                self.inventory.as_ref(),
                payment_slot,
                &mut receipt,
                source.get_max_stack_size(),
            );
            let leftover = crate::inventory::slot_transaction::refund(
                inventory.as_ref(),
                slot.get_index(),
                &source,
                receipt,
            );
            // Both admitted containers remain live, so an unaccepted payment always fits back.
            debug_assert!(leftover.is_empty());
            drop(guards);
            inventory.on_slot_changed(slot.get_index());
            self.inventory.on_slot_changed(payment_slot);
            let payment = self.inventory.get_stack(payment_slot);
            if payment.item_count == payment.get_max_stack_size() {
                break;
            }
        }
    }

    pub fn update_result_slot(&mut self) {
        let input_a = self.inventory.get_stack(0);
        let input_b = self.inventory.get_stack(1);
        self.active_offer = self.find_active_offer(&input_a, &input_b);

        if let Some(index) = self.active_offer {
            self.inventory
                .set_stack(2, (*self.offers[index].output.0).clone());
        } else {
            self.inventory.set_stack(2, ItemStack::EMPTY.clone());
        }
    }

    fn notify_trade_updated(&self, player: &dyn InventoryPlayer) {
        if self.inventory.get_stack(0).is_empty() && self.inventory.get_stack(1).is_empty() {
            return;
        }
        if let Some(on_trade_updated) = &self.on_trade_updated {
            // MerchantContainer.updateSellItem captures the result for notifyTradeUpdated.
            let callback = on_trade_updated.clone();
            let has_result = self.active_offer.is_some();
            player.run_menu_callback(Box::new(move || callback(has_result)));
        }
    }
}

impl ScreenHandler for MerchantScreenHandler {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn can_use(&self, player: &dyn InventoryPlayer) -> bool {
        self.validity_check
            .as_ref()
            .is_none_or(|check| check(player))
    }

    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        &self.behaviour
    }

    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        &mut self.behaviour
    }

    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        if let Some(on_close) = &self.on_close {
            let callback = on_close.clone();
            player.run_menu_callback(Box::new(move || callback()));
        }
        self.default_on_closed(player);
        self.inventory.on_close();
        // Vanilla drops items from merchant container on close
        for i in 0..2 {
            // Drop inputs only, output is virtual/ghost in some sense or just cleared
            let stack = self.inventory.remove_stack(i);
            if !stack.is_empty() {
                offer_or_drop_stack(player, stack);
            }
        }
        // Clear output slot
        self.inventory.set_stack(2, ItemStack::EMPTY.clone());
    }

    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        const PLAYER_INVENTORY_START: i32 = 3;
        const PLAYER_HOTBAR_START: i32 = 30;

        let slot = self.get_behaviour().slots[slot_index as usize].clone();
        let mut stack = slot.get_cloned_stack();
        if stack.is_empty() {
            return ItemStack::EMPTY.clone();
        }
        let mut original = stack.clone();
        let player_slots_end = self.get_behaviour().slots.len() as i32;

        if slot_index == 2 {
            if !self.can_fully_insert(
                &stack,
                PLAYER_INVENTORY_START as usize,
                player_slots_end as usize,
            ) || !self.transfer_item(
                player,
                slot.as_ref(),
                &mut stack,
                (PLAYER_INVENTORY_START, player_slots_end, true),
                &mut original,
            ) {
                return ItemStack::EMPTY.clone();
            }
        } else if slot_index < PLAYER_INVENTORY_START {
            if !self.transfer_item(
                player,
                slot.as_ref(),
                &mut stack,
                (PLAYER_INVENTORY_START, player_slots_end, false),
                &mut original,
            ) {
                return ItemStack::EMPTY.clone();
            }
        } else if slot_index < PLAYER_HOTBAR_START {
            if !self.transfer_item(
                player,
                slot.as_ref(),
                &mut stack,
                (PLAYER_HOTBAR_START, player_slots_end, false),
                &mut original,
            ) {
                return ItemStack::EMPTY.clone();
            }
        } else if !self.transfer_item(
            player,
            slot.as_ref(),
            &mut stack,
            (PLAYER_INVENTORY_START, PLAYER_HOTBAR_START, false),
            &mut original,
        ) {
            return ItemStack::EMPTY.clone();
        }

        if stack.item_count == original.item_count {
            return ItemStack::EMPTY.clone();
        }

        let mut taken = original.clone();
        taken.set_count(original.item_count - stack.item_count);
        slot.on_take_item(player, &taken);
        if slot_index == 2 {
            slot.on_quick_move_crafted(stack, original.clone());
            self.result_taken.store(false, Ordering::Relaxed);
            if !self.complete_trade(player) {
                return ItemStack::EMPTY.clone();
            }
            self.update_result_slot();
        }

        original
    }

    fn on_slot_click(
        &mut self,
        slot_index: i32,
        button: i32,
        action_type: pumpkin_protocol::java::server::play::SlotActionType,
        player: &dyn InventoryPlayer,
    ) {
        if slot_index == 2 {
            self.update_result_slot();
        }
        self.result_taken.store(false, Ordering::Relaxed);
        self.internal_on_slot_click(slot_index, button, action_type, player);
        if self.result_taken.swap(false, Ordering::Relaxed) {
            self.complete_trade(player);
        }
        self.update_result_slot();
        self.notify_trade_updated(player);
        self.send_content_updates();
    }
}

struct MerchantResultSlot {
    inventory: Arc<dyn Inventory>,
    index: usize,
    id: AtomicU8,
    result_taken: Arc<AtomicBool>,
}

impl MerchantResultSlot {
    fn new(inventory: Arc<dyn Inventory>, index: usize, result_taken: Arc<AtomicBool>) -> Self {
        Self {
            inventory,
            index,
            id: AtomicU8::new(0),
            result_taken,
        }
    }
}

impl Slot for MerchantResultSlot {
    fn withdrawal_count(&self, _amount: u8) -> u8 {
        u8::MAX
    }

    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }

    fn get_index(&self) -> usize {
        self.index
    }

    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }

    fn take_stack(&self, _amount: u8) -> ItemStack {
        self.inventory.remove_stack(self.index)
    }

    fn on_take_item(&self, _player: &dyn InventoryPlayer, _stack: &ItemStack) {
        self.result_taken.store(true, Ordering::Relaxed);
        self.mark_dirty();
    }

    fn can_insert(&self, _stack: &ItemStack) -> bool {
        false
    }

    fn mark_dirty(&self) {
        self.inventory.mark_dirty();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        borrow::Cow,
        sync::atomic::{AtomicI32, AtomicUsize, Ordering},
    };

    use crate::inventory::SimpleInventory;
    use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item};
    use pumpkin_protocol::{
        codec::item_stack_seralizer::ItemStackSerializer,
        java::{
            client::play::{
                CSetContainerContent, CSetContainerProperty, CSetContainerSlot, CSetCursorItem,
                CSetPlayerInventory, CSetSelectedSlot,
            },
            server::play::SlotActionType,
        },
    };
    use std::sync::Mutex;

    use crate::{entity_equipment::EntityEquipment, screen_handler::InventoryPlayer};

    use super::*;

    fn single_cost_offer(
        cost: &'static Item,
        cost_count: u8,
        output: &'static Item,
        max_uses: i32,
    ) -> pumpkin_protocol::java::client::play::MerchantOffer {
        pumpkin_protocol::java::client::play::MerchantOffer {
            base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(cost_count, cost))),
            output: ItemStackSerializer(Cow::Owned(ItemStack::new(1, output))),
            cost_b: None,
            reward_exp: true,
            uses: 0,
            max_uses,
            xp: 2,
            special_price: 0,
            price_multiplier: 0.05,
            demand: 0,
        }
    }

    fn bookshelf_offer() -> pumpkin_protocol::java::client::play::MerchantOffer {
        single_cost_offer(&Item::EMERALD, 9, &Item::BOOKSHELF, 12)
    }

    struct TestPlayer {
        inventory: Arc<PlayerInventory>,
        experience: AtomicI32,
        traded: AtomicI32,
        callbacks: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    }

    impl TestPlayer {
        fn flush_callbacks(&self) {
            let callbacks = std::mem::take(&mut *self.callbacks.lock().unwrap());
            for callback in callbacks {
                callback();
            }
        }

        fn new(inventory: Arc<PlayerInventory>) -> Self {
            Self {
                inventory,
                experience: AtomicI32::new(0),
                traded: AtomicI32::new(0),
                callbacks: Mutex::default(),
            }
        }
    }

    impl InventoryPlayer for TestPlayer {
        fn run_menu_callback(&self, callback: Box<dyn FnOnce() + Send>) {
            self.callbacks.lock().unwrap().push(callback);
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn drop_item(&self, _item: ItemStack, _retain_ownership: bool) {}

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
            0
        }

        fn add_experience_levels(&self, _levels: i32) {}

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

        fn award_experience(&self, amount: i32) {
            self.experience.fetch_add(amount, Ordering::Relaxed);
        }

        fn increment_stat(&self, category: StatisticCategory, stat_id: i32, amount: i32) {
            if category == StatisticCategory::Custom
                && stat_id == CustomStatistic::TradedWithVillager as i32
            {
                self.traded.fetch_add(amount, Ordering::Relaxed);
            }
        }

        fn play_block_sound(&self, _sound: pumpkin_data::sound::Sound, _pitch: f32) {}
    }

    fn inventories() -> (Arc<PlayerInventory>, Arc<SimpleInventory>) {
        (
            Arc::new(PlayerInventory::new(
                Arc::new(Mutex::new(EntityEquipment::new())),
                Arc::new(rustc_hash::FxHashMap::default()),
            )),
            Arc::new(SimpleInventory::new(3)),
        )
    }

    #[test]
    fn numeric_hotbar_key_swaps_occupied_stacks() {
        let (player_inventory, merchant_inventory) = inventories();
        player_inventory.set_stack(0, ItemStack::new(1, &Item::WOODEN_PICKAXE));
        player_inventory.set_stack(1, ItemStack::new(1, &Item::WOODEN_SWORD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler =
            MerchantScreenHandler::new(1, &player_inventory, merchant_inventory, Vec::new());

        // Merchant screen slots 30-38 map to hotbar slots 0-8. First move the
        // sword from hotbar slot 1 to the empty hotbar slot 2.
        handler.on_slot_click(31, 2, SlotActionType::Swap, &player);
        assert!(player_inventory.get_stack(1).is_empty());
        assert_eq!(player_inventory.get_stack(2).item.id, Item::WOODEN_SWORD.id);

        // Then reproduce #3059 by swapping that sword with the occupied slot 0.
        handler.on_slot_click(32, 0, SlotActionType::Swap, &player);
        assert_eq!(player_inventory.get_stack(0).item.id, Item::WOODEN_SWORD.id);
        assert_eq!(
            player_inventory.get_stack(2).item.id,
            Item::WOODEN_PICKAXE.id
        );
    }

    #[test]
    fn selecting_bookshelf_trade_moves_payment_and_creates_result() {
        let (player_inventory, merchant_inventory) = inventories();
        player_inventory.set_stack(0, ItemStack::new(12, &Item::EMERALD));
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![bookshelf_offer()],
        );

        let player = TestPlayer::new(player_inventory.clone());
        handler.set_selected_offer(0, &player);

        let payment = merchant_inventory.get_stack(0);
        assert_eq!(payment.item_count, 12);
        let result = merchant_inventory.get_stack(2);
        assert_eq!(result.item.id, Item::BOOKSHELF.id);
        assert!(player_inventory.get_stack(0).is_empty());
    }

    #[test]
    fn taking_result_commits_payment_after_delivery() {
        let (player_inventory, merchant_inventory) = inventories();
        merchant_inventory.set_stack(0, ItemStack::new(12, &Item::EMERALD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![bookshelf_offer()],
        );
        let trade_count = Arc::new(AtomicUsize::new(0));
        handler.on_trade = Some(Arc::new({
            let trade_count = trade_count.clone();
            move |_| {
                trade_count.fetch_add(1, Ordering::Relaxed);
            }
        }));
        handler.update_result_slot();

        handler.on_slot_click(2, 0, SlotActionType::Pickup, &player);

        assert_eq!(handler.offers[0].uses, 1);
        assert_eq!(merchant_inventory.get_stack(0).item_count, 3);
        assert_eq!(
            handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .item
                .id,
            Item::BOOKSHELF.id
        );
        assert_eq!(player.traded.load(Ordering::Relaxed), 1);
        player.flush_callbacks();
        assert_eq!(trade_count.load(Ordering::Relaxed), 1);
        assert_eq!(player.experience.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn bedrock_result_request_commits_the_trade() {
        let (player_inventory, merchant_inventory) = inventories();
        merchant_inventory.set_stack(0, ItemStack::new(12, &Item::EMERALD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![bookshelf_offer()],
        );
        let trade_count = Arc::new(AtomicUsize::new(0));
        handler.on_trade = Some(Arc::new({
            let trade_count = trade_count.clone();
            move |_| {
                trade_count.fetch_add(1, Ordering::Relaxed);
            }
        }));
        handler.update_result_slot();

        assert!(handler.complete_bedrock_trade(&player));

        assert_eq!(handler.offers[0].uses, 1);
        assert_eq!(merchant_inventory.get_stack(0).item_count, 3);
        assert_eq!(player.traded.load(Ordering::Relaxed), 1);
        player.flush_callbacks();
        assert_eq!(trade_count.load(Ordering::Relaxed), 1);
        assert!(merchant_inventory.get_stack(2).is_empty());
    }

    #[test]
    fn selection_hint_zero_resolves_matching_offer() {
        let (player_inventory, merchant_inventory) = inventories();
        merchant_inventory.set_stack(0, ItemStack::new(64, &Item::PAPER));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![
                bookshelf_offer(),
                single_cost_offer(&Item::PAPER, 24, &Item::EMERALD, 16),
            ],
        );
        let traded_offer = Arc::new(AtomicUsize::new(usize::MAX));
        handler.on_trade = Some(Arc::new({
            let traded_offer = traded_offer.clone();
            move |offer_index| {
                traded_offer.store(offer_index, Ordering::Relaxed);
            }
        }));
        handler.update_result_slot();

        handler.on_slot_click(2, 0, SlotActionType::Pickup, &player);

        assert_eq!(handler.offers[0].uses, 0);
        assert_eq!(handler.offers[1].uses, 1);
        player.flush_callbacks();
        assert_eq!(traded_offer.load(Ordering::Relaxed), 1);
        assert_eq!(merchant_inventory.get_stack(0).item_count, 40);
        assert_eq!(
            handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .item
                .id,
            Item::EMERALD.id
        );
    }

    #[test]
    fn invalid_payment_does_not_notify_the_merchant() {
        let (player_inventory, merchant_inventory) = inventories();
        merchant_inventory.set_stack(0, ItemStack::new(8, &Item::EMERALD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![bookshelf_offer()],
        );
        let trade_count = Arc::new(AtomicUsize::new(0));
        handler.on_trade = Some(Arc::new({
            let trade_count = trade_count.clone();
            move |_| {
                trade_count.fetch_add(1, Ordering::Relaxed);
            }
        }));

        handler.on_slot_click(2, 0, SlotActionType::Pickup, &player);

        assert_eq!(handler.offers[0].uses, 0);
        player.flush_callbacks();
        assert_eq!(trade_count.load(Ordering::Relaxed), 0);
        assert_eq!(player.traded.load(Ordering::Relaxed), 0);
        assert_eq!(merchant_inventory.get_stack(0).item_count, 8);
        assert!(
            handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn full_inventory_does_not_commit_quick_moved_trade() {
        let (player_inventory, merchant_inventory) = inventories();
        player_inventory
            .main_inventory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fill_with(|| ItemStack::new(64, &Item::COBBLESTONE));
        merchant_inventory.set_stack(0, ItemStack::new(9, &Item::EMERALD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![bookshelf_offer()],
        );
        handler.update_result_slot();

        handler.on_slot_click(2, 0, SlotActionType::QuickMove, &player);

        assert_eq!(handler.offers[0].uses, 0);
        assert_eq!(merchant_inventory.get_stack(0).item_count, 9);
        assert_eq!(merchant_inventory.get_stack(2).item.id, Item::BOOKSHELF.id);
        assert_eq!(player.traded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn quick_move_repeats_while_payment_and_stock_remain() {
        let (player_inventory, merchant_inventory) = inventories();
        merchant_inventory.set_stack(0, ItemStack::new(64, &Item::EMERALD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![bookshelf_offer()],
        );
        handler.update_result_slot();

        handler.on_slot_click(2, 0, SlotActionType::QuickMove, &player);

        assert_eq!(handler.offers[0].uses, 7);
        assert_eq!(player.traded.load(Ordering::Relaxed), 7);
        assert_eq!(merchant_inventory.get_stack(0).item_count, 1);
        assert_eq!(player_inventory.get_stack(8).item_count, 7);
    }

    #[test]
    fn swapped_payment_slots_are_accepted_and_consumed() {
        let (player_inventory, merchant_inventory) = inventories();
        let mut offer = bookshelf_offer();
        offer.base_cost_a = ItemStackSerializer(Cow::Owned(ItemStack::new(5, &Item::EMERALD)));
        offer.cost_b = Some(ItemStackSerializer(Cow::Owned(ItemStack::new(
            1,
            &Item::BOOK,
        ))));
        merchant_inventory.set_stack(0, ItemStack::new(1, &Item::BOOK));
        merchant_inventory.set_stack(1, ItemStack::new(5, &Item::EMERALD));
        let player = TestPlayer::new(player_inventory.clone());
        let mut handler = MerchantScreenHandler::new(
            1,
            &player_inventory,
            merchant_inventory.clone(),
            vec![offer],
        );
        handler.update_result_slot();

        handler.on_slot_click(2, 0, SlotActionType::Pickup, &player);

        assert_eq!(handler.offers[0].uses, 1);
        assert!(merchant_inventory.get_stack(0).is_empty());
        assert!(merchant_inventory.get_stack(1).is_empty());
    }
}
