use std::any::Any;
use std::sync::Arc;

use crate::inventory::Inventory;
use pumpkin_data::Enchantment;
use pumpkin_data::data_component_impl::EnchantableImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::screen::WindowType;
use pumpkin_data::tag::{Enchantment as EnchantmentTag, Taggable};
use pumpkin_util::random::{RandomImpl, legacy_rand::LegacyRand};

use crate::{
    player::player_inventory::PlayerInventory,
    screen_handler::{InventoryPlayer, ScreenHandler, ScreenHandlerBehaviour, offer_or_drop_stack},
    slot::{NormalSlot, Slot},
    window_property::{EnchantmentTable, WindowProperty},
};

struct LapisSlot(NormalSlot);

fn is_lapis(stack: &ItemStack) -> bool {
    stack.item == &Item::LAPIS_LAZULI
}

impl LapisSlot {
    fn new(inventory: Arc<dyn Inventory>) -> Self {
        Self(NormalSlot::new(inventory, 1))
    }
}

impl Slot for LapisSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.0.get_inventory()
    }

    fn get_index(&self) -> usize {
        self.0.get_index()
    }

    fn set_id(&self, id: usize) {
        self.0.set_id(id);
    }

    fn can_insert(&self, stack: &ItemStack) -> bool {
        is_lapis(stack)
    }

    fn mark_dirty(&self) {
        self.0.mark_dirty();
    }
}

pub struct EnchantingTableScreenHandler {
    pub inventory: Arc<dyn Inventory>,
    behaviour: ScreenHandlerBehaviour,
    pub level_requirements: [i32; 3],
    pub enchantment_id: [i32; 3],
    pub enchantment_level: [i32; 3],
    pub enchantment_seed: i32,
    pub bookshelf_count: i32,
    pub(super) offers_pending: bool,
    pub(super) offer_revision: u64,
}

impl EnchantingTableScreenHandler {
    pub fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        inventory: &Arc<dyn Inventory>,
        enchantment_seed: i32,
        bookshelf_count: i32,
    ) -> Self {
        let mut handler = Self {
            inventory: inventory.clone(),
            behaviour: ScreenHandlerBehaviour::new(sync_id, Some(WindowType::Enchantment)),
            level_requirements: [0; 3],
            enchantment_id: [-1; 3],
            enchantment_level: [-1; 3],
            enchantment_seed,
            bookshelf_count,
            offers_pending: false,
            offer_revision: 0,
        };

        // Enchanting slots: 0 is item, 1 is lapis
        handler.add_slot(Arc::new(NormalSlot::new(inventory.clone(), 0)));
        handler.add_slot(Arc::new(LapisSlot::new(inventory.clone())));

        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();
        handler.add_player_slots(&player_inventory);

        handler
    }

    pub fn update_enchantments(&mut self, _player: &dyn InventoryPlayer) {
        self.offer_revision = self.offer_revision.wrapping_add(1);
        self.offers_pending = false;
        let item = self.inventory.get_stack(0);

        if item.is_empty() || item.has_enchantments() {
            for i in 0..3 {
                self.level_requirements[i] = 0;
                self.enchantment_id[i] = -1;
                self.enchantment_level[i] = -1;
            }
        } else {
            let enchantability = item
                .get_data_component::<EnchantableImpl>()
                .map_or(0, |e| e.value);

            if enchantability <= 0 {
                for i in 0..3 {
                    self.level_requirements[i] = 0;
                    self.enchantment_id[i] = -1;
                    self.enchantment_level[i] = -1;
                }
            } else {
                let mut random = LegacyRand::from_seed(self.enchantment_seed as u64);

                for i in 0..3 {
                    let level = self.calculate_level_requirement(&mut random, i);
                    self.level_requirements[i] = level;
                }

                for i in 0..3 {
                    if self.level_requirements[i] > 0 {
                        let mut random = self.create_enchantment_random(i);
                        let enchantments = Self::get_enchantment_list(
                            &mut random,
                            &item,
                            self.level_requirements[i],
                        );
                        if enchantments.is_empty() {
                            self.enchantment_id[i] = -1;
                            self.enchantment_level[i] = -1;
                        } else {
                            let clue_index =
                                random.next_bounded_i32(enchantments.len() as i32) as usize;
                            let clue = enchantments[clue_index];
                            self.enchantment_id[i] = clue.0.id as i32;
                            self.enchantment_level[i] = clue.1;
                        }
                    } else {
                        self.enchantment_id[i] = -1;
                        self.enchantment_level[i] = -1;
                    }
                }

                self.offers_pending = true;
            }
        }
        if !self.offers_pending {
            self.send_property_updates();
        }
    }

    fn calculate_level_requirement(&self, random: &mut LegacyRand, slot: usize) -> i32 {
        let b = self.bookshelf_count;
        let level = random.next_bounded_i32(8) + 1 + (b >> 1) + random.next_bounded_i32(b + 1);

        // EnchantmentHelper.getEnchantmentCost uses selected * 2 / 3 + 1 for slot 1.
        match slot {
            0 => (level / 3).max(1),
            1 => (level * 2 / 3 + 1).max(1),
            2 => level.max(b * 2).max(1),
            _ => 0,
        }
    }

    pub(super) const fn create_enchantment_random(&self, slot: usize) -> LegacyRand {
        LegacyRand::from_seed(self.enchantment_seed.wrapping_add(slot as i32) as u64)
    }

    pub(super) fn get_enchantment_list(
        random: &mut LegacyRand,
        item: &ItemStack,
        level: i32,
    ) -> Vec<(&'static Enchantment, i32)> {
        let enchantability = item
            .get_data_component::<EnchantableImpl>()
            .map_or(0, |e| e.value);
        let mut enchant_level = level
            + 1
            + random.next_bounded_i32(enchantability / 4 + 1)
            + random.next_bounded_i32(enchantability / 4 + 1);
        let bonus = (random.next_f32() + random.next_f32() - 1.0) * 0.15;
        enchant_level = (enchant_level as f32 * (1.0 + bonus)).round() as i32;
        enchant_level = enchant_level.max(1);

        let mut available = Vec::new();
        for enchant in Enchantment::all() {
            // EnchantmentHelper.getAvailableEnchantmentResults admits primary items or a regular book.
            if enchant.has_tag(&EnchantmentTag::MINECRAFT_IN_ENCHANTING_TABLE)
                && (enchant.is_primary_item(item.item) || item.item == &Item::BOOK)
            {
                for l in (1..=enchant.max_level).rev() {
                    if enchant_level >= enchant.min_cost.calculate(l)
                        && enchant_level <= enchant.max_cost.calculate(l)
                    {
                        available.push((*enchant, l));
                        break;
                    }
                }
            }
        }

        if available.is_empty() {
            return Vec::new();
        }

        let total_weight: i32 = available.iter().map(|(e, _)| e.weight).sum();
        if total_weight <= 0 {
            return Vec::new();
        }

        let mut weight = random.next_bounded_i32(total_weight);
        let mut selected = None;
        for (e, l) in &available {
            weight -= e.weight;
            if weight < 0 {
                selected = Some((*e, *l));
                break;
            }
        }

        let mut result = Vec::new();
        if let Some(s) = selected {
            result.push(s);

            // Add more?
            let mut current_level = enchant_level;
            while random.next_bounded_i32(50) <= (current_level + 1) / 2 {
                available.retain(|(e, _)| {
                    for (se, _) in &result {
                        if !e.are_compatible(se) {
                            return false;
                        }
                    }
                    true
                });

                if available.is_empty() {
                    break;
                }

                let total_weight: i32 = available.iter().map(|(e, _)| e.weight).sum();
                let mut weight = random.next_bounded_i32(total_weight);
                for (e, l) in &available {
                    weight -= e.weight;
                    if weight < 0 {
                        result.push((*e, *l));
                        break;
                    }
                }
                current_level /= 2;
            }
        }

        if item.item == &Item::BOOK && result.len() > 1 {
            let removed_index = random.next_bounded_i32(result.len() as i32) as usize;
            result.remove(removed_index);
        }

        result
    }

    pub(super) fn send_property_updates(&self) {
        if let Some(sync_handler) = self.behaviour.sync_handler.as_ref() {
            for i in 0..3 {
                let (id, val) = WindowProperty::new(
                    EnchantmentTable::LevelRequirement { slot: i as u8 },
                    self.level_requirements[i] as i16,
                )
                .into_tuple();
                sync_handler.update_property(&self.behaviour, id as i32, val as i32);

                let (id, val) = WindowProperty::new(
                    EnchantmentTable::EnchantmentId { slot: i as u8 },
                    self.enchantment_id[i] as i16,
                )
                .into_tuple();
                sync_handler.update_property(&self.behaviour, id as i32, val as i32);

                let (id, val) = WindowProperty::new(
                    EnchantmentTable::EnchantmentLevel { slot: i as u8 },
                    self.enchantment_level[i] as i16,
                )
                .into_tuple();
                sync_handler.update_property(&self.behaviour, id as i32, val as i32);
            }

            let (id, val) = WindowProperty::new(
                EnchantmentTable::EnchantmentSeed,
                (self.enchantment_seed & 0xFFFF) as i16,
            )
            .into_tuple();
            sync_handler.update_property(&self.behaviour, id as i32, val as i32);
        }
    }
}

impl ScreenHandler for EnchantingTableScreenHandler {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        &self.behaviour
    }

    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        &mut self.behaviour
    }

    fn on_closed(&mut self, player: &dyn InventoryPlayer) {
        self.default_on_closed(player);
        self.inventory.on_close();
        // Return items to player
        for i in 0..2 {
            let stack = self.inventory.remove_stack(i);
            if !stack.is_empty() {
                offer_or_drop_stack(player, stack);
            }
        }
    }

    fn on_button_click(&mut self, player: &dyn InventoryPlayer, id: i32) -> bool {
        // Unlocked callers may use this convenience path; Player splits the callback.
        let Some(mut proposal) = self.prepare_enchantment(player, id) else {
            return false;
        };
        if player.fire_enchant_item_event(
            &proposal.item,
            id,
            proposal.level_requirement,
            &mut proposal.enchantments,
        ) {
            return false;
        }
        self.commit_enchantment(player, proposal)
    }

    fn quick_move(&mut self, player: &dyn InventoryPlayer, slot_index: i32) -> ItemStack {
        let mut stack_left = ItemStack::EMPTY.clone();
        let slot = self.get_behaviour().slots[slot_index as usize].clone();

        if slot.has_stack() {
            let mut slot_stack = slot.get_stack();
            stack_left = slot_stack.clone();

            if slot_index < 2 {
                // From enchanting to player
                if !self.transfer_item(
                    player,
                    slot.as_ref(),
                    &mut slot_stack,
                    (2, self.get_behaviour().slots.len() as i32, true),
                    &mut stack_left,
                ) {
                    return ItemStack::EMPTY.clone();
                }
            } else {
                // From player to enchanting
                // Lapis check
                if slot_stack.item == &Item::LAPIS_LAZULI {
                    if !self.transfer_item(
                        player,
                        slot.as_ref(),
                        &mut slot_stack,
                        (1, 2, false),
                        &mut stack_left,
                    ) {
                        return ItemStack::EMPTY.clone();
                    }
                } else if !self.transfer_item(
                    player,
                    slot.as_ref(),
                    &mut slot_stack,
                    (0, 1, false),
                    &mut stack_left,
                ) {
                    return ItemStack::EMPTY.clone();
                }
            }

            // CRITICAL FIX: Ensure the client is notified when shift-clicking items into the slots
            self.update_enchantments(player);
        }

        stack_left
    }

    fn on_slot_click(
        &mut self,
        slot_index: i32,
        button: i32,
        action_type: pumpkin_protocol::java::server::play::SlotActionType,
        player: &dyn InventoryPlayer,
    ) {
        self.internal_on_slot_click(slot_index, button, action_type, player);
        if slot_index == 0 || slot_index == 1 {
            self.update_enchantments(player);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lapis_slot_only_accepts_lapis_lazuli() {
        assert!(is_lapis(&ItemStack::new(1, &Item::LAPIS_LAZULI)));
        assert!(!is_lapis(&ItemStack::new(1, &Item::DIRT)));
    }
}
