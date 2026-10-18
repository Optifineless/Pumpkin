use pumpkin_data::{
    Enchantment,
    data_component_impl::StoredEnchantmentsImpl,
    item::Item,
    item_stack::ItemStack,
    sound::Sound,
    statistic::{CustomStatistic, StatisticCategory},
};

use super::enchanting_screen_handler::EnchantingTableScreenHandler;
use crate::screen_handler::{InventoryPlayer, ScreenHandler};
use std::borrow::Cow;

/// An enchantment proposal whose inputs must still match after the plugin callback.
pub struct EnchantmentProposal {
    pub item: ItemStack,
    lapis: ItemStack,
    pub option: i32,
    pub level_requirement: i32,
    pub enchantments: Vec<(&'static Enchantment, i32)>,
    seed: i32,
    revision: u64,
}

/// A snapshot of generated offers for the cancellable preparation callback.
pub struct EnchantmentOffers {
    pub item: ItemStack,
    pub costs: [i32; 3],
    pub ids: [i32; 3],
    pub levels: [i32; 3],
    pub bookshelves: i32,
    seed: i32,
    revision: u64,
}

impl EnchantingTableScreenHandler {
    /// Captures the inputs to EnchantmentMenu.clickMenuButton before invoking plugins.
    pub fn prepare_enchantment(
        &self,
        player: &dyn InventoryPlayer,
        option: i32,
    ) -> Option<EnchantmentProposal> {
        let index = usize::try_from(option)
            .ok()
            .filter(|&index| index < self.level_requirements.len())?;
        let cost = self.level_requirements[index];
        let lapis = self.inventory.get_stack(1);
        let item = self.inventory.get_stack(0);
        if cost <= 0
            || item.is_empty()
            || item.has_enchantments()
            || (!player.is_creative()
                && (player.experience_level() < cost
                    || player.experience_level() < option + 1
                    || lapis.item != &Item::LAPIS_LAZULI
                    || i32::from(lapis.item_count) < option + 1))
        {
            return None;
        }
        let mut random = self.create_enchantment_random(index);
        let enchantments = Self::get_enchantment_list(&mut random, &item, cost);
        if enchantments.is_empty() {
            return None;
        }
        Some(EnchantmentProposal {
            item,
            lapis,
            option,
            level_requirement: cost,
            enchantments,
            seed: self.enchantment_seed,
            revision: self.offer_revision,
        })
    }

    /// Commits only if every menu input and the player's current resources remain valid.
    pub fn commit_enchantment(
        &mut self,
        player: &dyn InventoryPlayer,
        proposal: EnchantmentProposal,
    ) -> bool {
        // EnchantmentMenu.clickMenuButton validates resources before charging or changing items.
        let Some(index) = usize::try_from(proposal.option)
            .ok()
            .filter(|&index| index < self.level_requirements.len())
        else {
            return false;
        };
        if proposal.enchantments.is_empty()
            || self.offer_revision != proposal.revision
            || self.enchantment_seed != proposal.seed
            || player.enchantment_seed() != proposal.seed
            || self.level_requirements[index] != proposal.level_requirement
            || !self.inventory.get_stack(0).are_equal(&proposal.item)
            || !self.inventory.get_stack(1).are_equal(&proposal.lapis)
            || player.is_removed()
            || (!player.is_creative()
                && (player.experience_level() < proposal.level_requirement
                    || player.experience_level() < proposal.option + 1))
        {
            return false;
        }
        let mut item = proposal.item.clone();
        let mut lapis = proposal.lapis.clone();
        if item.item == &Item::BOOK {
            // EnchantmentMenu.clickMenuButton transmutes BOOK and stores its rolled enchantments.
            item.item = &Item::ENCHANTED_BOOK;
            item.set_data_component(StoredEnchantmentsImpl {
                enchantment: Cow::Owned(proposal.enchantments),
            });
        } else {
            for (enchantment, level) in proposal.enchantments {
                item.add_enchantment(enchantment, level as u16);
            }
        }
        if !player.is_creative() {
            lapis.decrement((proposal.option + 1) as u8);
        }
        if !self.commit_enchantment_inputs(&proposal.item, &proposal.lapis, item, lapis) {
            return false;
        }
        if !player.is_creative() {
            player.add_experience_levels(-(proposal.option + 1));
        }
        player.set_enchantment_seed(rand::random());
        player.increment_stat(
            StatisticCategory::Custom,
            CustomStatistic::EnchantItem as i32,
            1,
        );
        self.enchantment_seed = player.enchantment_seed();
        self.update_enchantments(player);
        self.send_content_updates();
        player.play_block_sound(
            Sound::BlockEnchantmentTableUse,
            rand::random::<f32>().mul_add(0.1, 0.9),
        );
        true
    }

    // EnchantmentMenu.clickMenuButton commits the enchanted item and its lapis together.
    fn commit_enchantment_inputs(
        &self,
        expected_item: &ItemStack,
        expected_lapis: &ItemStack,
        item: ItemStack,
        lapis: ItemStack,
    ) -> bool {
        let owner = self.inventory.slot_transaction(0);
        let guard = owner.lock();
        let current_item = self.inventory.get_stack(0);
        let current_lapis = self.inventory.get_stack(1);
        if !owner.is_live()
            || current_item.uid != expected_item.uid
            || !current_item.are_equal(expected_item)
            || current_lapis.uid != expected_lapis.uid
            || !current_lapis.are_equal(expected_lapis)
        {
            return false;
        }
        for (index, expected, replacement) in [(0, expected_item, item), (1, expected_lapis, lapis)]
        {
            let committed = crate::inventory::slot_transaction::exchange(
                self.inventory.as_ref(),
                index,
                expected,
                replacement,
            );
            debug_assert!(committed);
        }
        drop(guard);
        self.inventory.on_slot_changed(0);
        self.inventory.on_slot_changed(1);
        true
    }

    /// Takes a pending preparation request without invoking the player under a menu guard.
    pub fn take_prepare_offers(&mut self) -> Option<EnchantmentOffers> {
        if !std::mem::take(&mut self.offers_pending) {
            return None;
        }
        Some(EnchantmentOffers {
            item: self.inventory.get_stack(0),
            costs: self.level_requirements,
            ids: self.enchantment_id,
            levels: self.enchantment_level,
            bookshelves: self.bookshelf_count,
            seed: self.enchantment_seed,
            revision: self.offer_revision,
        })
    }

    /// Applies a callback's offers only to the menu revision that generated them.
    pub fn apply_prepare_offers(&mut self, offers: &EnchantmentOffers, cancelled: bool) {
        // EnchantmentMenu.slotsChanged publishes clues for the current item and seed.
        if self.offer_revision != offers.revision
            || self.enchantment_seed != offers.seed
            || !self.inventory.get_stack(0).are_equal(&offers.item)
        {
            return;
        }
        self.level_requirements = if cancelled { [0; 3] } else { offers.costs };
        self.enchantment_id = if cancelled { [-1; 3] } else { offers.ids };
        self.enchantment_level = if cancelled { [-1; 3] } else { offers.levels };
        self.send_property_updates();
    }
}
