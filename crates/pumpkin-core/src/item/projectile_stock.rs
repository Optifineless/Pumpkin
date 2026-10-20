use crate::{entity::player::Player, item::item_utils::give_or_drop};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_inventory::{
    Inventory, inventory::slot_transaction, player::player_inventory::PlayerInventory,
};
use pumpkin_util::Hand;

/// Owns only the paid stock of one projectile attempt, never inventory ownership.
pub struct ProjectileStock {
    pub source: usize,
    original: ItemStack,
    pub receipt: ItemStack,
}

impl ProjectileStock {
    pub fn claim_hand(player: &Player, hand: Hand, expected: &ItemStack) -> Option<Self> {
        let source = match hand {
            Hand::Right => usize::from(player.inventory().get_selected_slot()),
            Hand::Left => PlayerInventory::OFF_HAND_SLOT,
        };
        Self::claim(player, source, expected, 1)
    }

    // Block item hooks have an observed stack but no hand argument.
    pub fn claim_observed(player: &Player, expected: &ItemStack) -> Option<Self> {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(0);
        let _guard = owner.lock();
        let source = (0..inventory.size()).find(|slot| {
            let stack = inventory.get_stack(*slot);
            stack.uid == expected.uid && stack.are_equal(expected)
        })?;
        Self::claim(player, source, expected, 1)
    }

    pub fn claim(player: &Player, source: usize, expected: &ItemStack, amount: u8) -> Option<Self> {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(source);
        let _guard = owner.lock();
        let current = inventory.get_stack(source);
        if !owner.is_live()
            || expected.is_empty()
            || current.uid != expected.uid
            || !current.are_equal(expected)
        {
            return None;
        }
        let receipt = if amount == 0 || player.has_infinite_materials() {
            ItemStack::EMPTY.clone()
        } else {
            let receipt = slot_transaction::withdraw(inventory.as_ref(), source, expected, amount);
            if receipt.is_empty() {
                return None;
            }
            receipt
        };
        Some(Self {
            source,
            original: expected.clone(),
            receipt,
        })
    }

    pub fn publish(&self, player: &Player) {
        player.sync_hand_slot(self.source, player.inventory().get_stack(self.source));
    }

    pub fn refund(mut self, player: &Player) {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(self.source);
        let guard = owner.lock();
        if owner.is_live() && !self.receipt.is_empty() {
            let current = inventory.get_stack(self.source);
            // Restore a last item with its original UID; never replace callback stock.
            if current.is_empty() && self.original.item_count == self.receipt.item_count {
                if slot_transaction::exchange(
                    inventory.as_ref(),
                    self.source,
                    &current,
                    self.original.clone(),
                ) {
                    self.receipt = ItemStack::EMPTY.clone();
                }
            } else if current.uid == self.original.uid
                && current.are_items_and_components_equal(&self.original)
            {
                self.receipt = slot_transaction::refund(
                    inventory.as_ref(),
                    self.source,
                    &self.original,
                    self.receipt,
                );
            }
        }
        drop(guard);
        // Existing inventory insertion/drop ownership accounts for every unreturned item.
        if !self.receipt.is_empty() {
            give_or_drop(player, self.receipt);
        }
        player.sync_hand_slot(self.source, inventory.get_stack(self.source));
    }
}
