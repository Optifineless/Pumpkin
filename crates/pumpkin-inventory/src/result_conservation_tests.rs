use crate::{
    Inventory, SimpleInventory,
    crafting::crafting_screen_handler::CraftingTableScreenHandler,
    ext_review_tests::Player,
    inventory::{Clearable, slot_transaction},
    merchant::merchant_screen_handler::MerchantScreenHandler,
    player::player_screen_handler::PlayerScreenHandler,
    screen_handler::ScreenHandler,
    slot::NormalSlot,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer,
    java::{client::play::MerchantOffer, server::play::SlotActionType},
};
use std::{any::Any, borrow::Cow, sync::Arc};

// A retained native inventory writer runs at the destination notification boundary.
// This is after delivery but before the old quick-move onTake payment, without a test hook.
struct RemovePaymentOnDelivery {
    backing: Arc<dyn Inventory>,
    payment: Arc<dyn Inventory>,
    stolen: Arc<SimpleInventory>,
}

impl Clearable for RemovePaymentOnDelivery {
    fn clear(&self) {
        self.backing.clear();
    }
}

impl Inventory for RemovePaymentOnDelivery {
    fn slot_transaction(&self, index: usize) -> Arc<slot_transaction::Transaction> {
        self.backing.slot_transaction(index)
    }
    fn collect_transaction_owners(&self, owners: &mut slot_transaction::Owners) {
        self.backing.collect_transaction_owners(owners);
    }
    fn update_slot_storage(&self, index: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        self.backing.update_slot_storage(index, update);
    }
    fn on_slot_replaced(&self, index: usize, before: &ItemStack, after: &ItemStack) {
        self.backing.on_slot_replaced(index, before, after);
    }
    fn on_slot_changed(&self, index: usize) {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let owners = slot_transaction::ordered_owners(&[
                        self.backing.as_ref(),
                        self.payment.as_ref(),
                    ]);
                    for owner in &owners {
                        assert!(owner.try_lock().is_some(), "notification retained an owner");
                    }
                    let mut removed = self.payment.remove_stack(0);
                    slot_transaction::deposit(self.stolen.as_ref(), 0, &mut removed, u8::MAX);
                    assert!(removed.is_empty());
                })
                .join()
                .unwrap();
        });
        self.backing.on_slot_changed(index);
    }
    fn size(&self) -> usize {
        self.backing.size()
    }
    fn is_empty(&self) -> bool {
        self.backing.is_empty()
    }
    fn get_stack(&self, index: usize) -> ItemStack {
        self.backing.get_stack(index)
    }
    fn remove_stack(&self, index: usize) -> ItemStack {
        self.backing.remove_stack(index)
    }
    fn remove_stack_specific(&self, index: usize, amount: u8) -> ItemStack {
        self.backing.remove_stack_specific(index, amount)
    }
    fn set_stack(&self, index: usize, stack: ItemStack) {
        self.backing.set_stack(index, stack);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn install_writer(
    menu: &mut dyn ScreenHandler,
    player: &Player,
    start: usize,
    end: usize,
    payment: Arc<dyn Inventory>,
) -> Arc<SimpleInventory> {
    let stolen = Arc::new(SimpleInventory::new(1));
    let target: Arc<dyn Inventory> = Arc::new(RemovePaymentOnDelivery {
        backing: player.inventory.clone(),
        payment,
        stolen: stolen.clone(),
    });
    for slot in &mut menu.get_behaviour_mut().slots[start..end] {
        *slot = Arc::new(NormalSlot::new(target.clone(), slot.get_index()));
    }
    stolen
}

fn crafted_count(player: &Player, item: &Item) -> u32 {
    (0..player.inventory.size())
        .map(|i| player.inventory.get_stack(i))
        .filter(|stack| stack.item == item && !stack.is_empty())
        .map(|stack| u32::from(stack.item_count))
        .sum()
}

#[test]
fn java_conservation_crafting_quick_move_pays_before_grid_writer() {
    for table in [true, false] {
        let player = Player::new(1);
        let mut menu: Box<dyn ScreenHandler> = if table {
            Box::new(CraftingTableScreenHandler::new(1, &player.inventory, None))
        } else {
            Box::new(PlayerScreenHandler::new(&player.inventory, None, 0, None))
        };
        let grid = menu.get_behaviour().slots[1].get_inventory();
        grid.set_stack(0, ItemStack::new(1, &Item::OAK_LOG));
        menu.get_behaviour().slots[0].set_stack(ItemStack::EMPTY.clone());
        let stolen = install_writer(
            menu.as_mut(),
            &player,
            if table { 10 } else { 9 },
            if table { 46 } else { 45 },
            grid.clone(),
        );
        menu.on_slot_click(0, 0, SlotActionType::QuickMove, &player);
        assert_eq!(crafted_count(&player, &Item::OAK_PLANKS), 4);
        assert!(grid.get_stack(0).is_empty());
        assert!(
            stolen.get_stack(0).is_empty(),
            "output delivered while the log escaped payment"
        );
        assert!(menu.get_behaviour().slots[0].get_stack().is_empty());
    }
}

#[test]
fn java_conservation_merchant_quick_move_pays_before_payment_writer() {
    let player = Player::new(1);
    let payment = Arc::new(SimpleInventory::new(3));
    payment.set_stack(0, ItemStack::new(9, &Item::EMERALD));
    let offer = MerchantOffer {
        base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(9, &Item::EMERALD))),
        output: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::BOOKSHELF))),
        cost_b: None,
        reward_exp: true,
        uses: 0,
        max_uses: 12,
        xp: 2,
        special_price: 0,
        price_multiplier: 0.05,
        demand: 0,
    };
    let mut menu = MerchantScreenHandler::new(1, &player.inventory, payment.clone(), vec![offer]);
    let stolen = install_writer(&mut menu, &player, 3, 39, payment.clone());
    menu.on_slot_click(2, 0, SlotActionType::QuickMove, &player);
    assert_eq!(crafted_count(&player, &Item::BOOKSHELF), 1);
    assert!(payment.get_stack(0).is_empty());
    assert!(
        stolen.get_stack(0).is_empty(),
        "output delivered while emeralds escaped payment"
    );
    assert_eq!(menu.offers[0].uses, 1);
}

// Admission predicates are a module boundary and may observe a competing hotbar edit.
struct EditHotbarBeforeAdmission {
    slot: Arc<dyn crate::slot::Slot>,
    effects: Arc<SimpleInventory>,
}
impl crate::slot::Slot for EditHotbarBeforeAdmission {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.slot.get_inventory()
    }
    fn get_index(&self) -> usize {
        self.slot.get_index()
    }
    fn set_id(&self, id: usize) {
        self.slot.set_id(id);
    }
    fn mark_dirty(&self) {
        self.slot.mark_dirty();
    }
    fn can_take_items(&self, player: &dyn crate::screen_handler::InventoryPlayer) -> bool {
        player
            .get_inventory()
            .set_stack(0, ItemStack::new(1, &Item::STONE));
        self.slot.can_take_items(player)
    }
    fn can_insert(&self, stack: &ItemStack) -> bool {
        self.slot.can_insert(stack)
    }
    fn on_crafted_by(
        &self,
        player: &dyn crate::screen_handler::InventoryPlayer,
        stack: &mut ItemStack,
    ) {
        self.slot.on_crafted_by(player, stack);
        self.effects.set_stack(0, ItemStack::new(1, &Item::PAPER));
    }
}

#[test]
fn java_conservation_rejected_cartography_swap_does_not_post_process() {
    use crate::cartography_table_screen_handler::CartographyTableScreenHandler;
    use pumpkin_data::data_component_impl::MapIdImpl;
    let player = Player::new(1);
    let mut menu = CartographyTableScreenHandler::new(1, &player.inventory);
    let mut map = ItemStack::new(1, &Item::FILLED_MAP);
    map.set_data_component(MapIdImpl { id: 0 });
    menu.input_inventory.set_stack(0, map);
    menu.input_inventory
        .set_stack(1, ItemStack::new(1, &Item::PAPER));
    menu.slots_changed(&player);
    let expected = menu.get_behaviour().slots[2].get_stack();
    assert!(!expected.is_empty());
    let effects = Arc::new(SimpleInventory::new(1));
    menu.get_behaviour_mut().slots[2] = Arc::new(EditHotbarBeforeAdmission {
        slot: menu.get_behaviour().slots[2].clone(),
        effects: effects.clone(),
    });
    menu.on_slot_click(2, 0, SlotActionType::Swap, &player);
    assert!(effects.is_empty(), "rejected swap ran map post-processing");
    assert!(menu.output_inventory.get_stack(0).are_equal(&expected));
    assert_eq!(player.inventory.get_stack(0).item, &Item::STONE);
    assert_eq!(menu.input_inventory.get_stack(1).item_count, 1);
}
