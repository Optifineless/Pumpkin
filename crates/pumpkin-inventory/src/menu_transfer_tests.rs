use crate::{
    Inventory, SimpleInventory,
    ext_review_tests::Player,
    inventory::slot_transaction,
    screen_handler::InventoryPlayer,
    slot::{ArmorSlot, NormalSlot, Slot},
};
use pumpkin_data::{
    Enchantment, data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

struct CheckedArmor {
    inner: ArmorSlot,
    owners: slot_transaction::Owners,
    calls: Arc<AtomicUsize>,
}
impl Slot for CheckedArmor {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inner.get_inventory()
    }
    fn get_index(&self) -> usize {
        self.inner.get_index()
    }
    fn set_id(&self, id: usize) {
        self.inner.set_id(id);
    }
    fn mark_dirty(&self) {
        self.inner.mark_dirty();
    }
    fn can_take_stack(&self, player: &dyn InventoryPlayer, stack: &ItemStack) -> bool {
        self.calls.fetch_add(1, SeqCst);
        let owners = self.owners.clone();
        let (done, result) = std::sync::mpsc::channel();
        let _probe = std::thread::spawn(move || {
            done.send(owners.iter().all(|owner| owner.try_lock().is_none()))
                .unwrap();
        });
        assert!(
            result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            "source take rule must run under both owners"
        );
        self.inner.can_take_stack(player, stack)
    }
}

#[test]
fn b1020c_g02_player_transfer_checks_pinned_source_and_preserves_legacy_api() {
    let player = Player::new(0);
    let target: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(1));
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    helmet.add_enchantment(&Enchantment::BINDING_CURSE, 1);
    player.inventory.set_stack(39, helmet.clone());
    let owners = slot_transaction::ordered_owners(&[
        player.inventory.as_ref() as &dyn Inventory,
        target.as_ref(),
    ]);
    let calls = Arc::new(AtomicUsize::new(0));
    let source = CheckedArmor {
        inner: ArmorSlot::new(player.inventory.clone(), 39, EquipmentSlot::HEAD),
        owners,
        calls: calls.clone(),
    };
    let slots: Vec<Arc<dyn Slot>> = vec![Arc::new(NormalSlot::new(target.clone(), 0))];
    let mut observed = helmet.clone();
    let mut before = helmet.clone();
    let (moved, leftover) = super::menu_transfer::transfer_for_player(
        &slots,
        &source,
        &mut observed,
        (0, 1, false),
        &mut before,
        &player,
    );
    assert!(!moved);
    assert!(leftover.is_empty());
    assert_eq!(calls.load(SeqCst), 1);
    assert!(observed.are_equal(&helmet) && before.are_equal(&helmet));
    assert_eq!(player.inventory.get_stack(39).uid, helmet.uid);
    assert!(target.get_stack(0).is_empty());
    // Existing merchant auto-payment keeps its unchecked, source-compatible entry point.
    let (moved, leftover) =
        super::menu_transfer::transfer(&slots, &source, &mut observed, (0, 1, false), &mut before);
    assert!(moved);
    assert!(leftover.is_empty());
    assert_eq!(calls.load(SeqCst), 1);
    assert!(player.inventory.get_stack(39).is_empty());
    assert!(target.get_stack(0).are_equal(&helmet));
}
