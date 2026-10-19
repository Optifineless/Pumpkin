use std::sync::atomic::Ordering;

use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::java::client::play::MerchantOffer;

use super::VillagerEntity;
use crate::entity::mob::Mob;

impl VillagerEntity {
    pub(super) fn write_villager_nbt(&self, nbt: &mut NbtCompound) {
        // Capture each owner separately; item codecs run after all guards drop.
        let offers = self
            .offers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let inventory = self
            .inventory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let gossips = self.snapshot_gossips();
        let job_site = self.get_job_site();
        let home = self.get_home();

        self.write_villager_data_nbt(nbt);
        self.write_food_nbt(nbt);
        nbt.put_int("Xp", self.xp.load(Ordering::Relaxed));
        nbt.put_long(
            "LastRestock",
            self.last_restock_time.load(Ordering::Relaxed),
        );
        nbt.put_int("RestocksToday", self.restocks_today.load(Ordering::Relaxed));
        nbt.put_long(
            "LastGossipDecay",
            self.last_gossip_decay_time.load(Ordering::Relaxed),
        );
        let last_share = self.last_gossip_share_time.load(Ordering::Relaxed);
        if last_share > 0 {
            nbt.put_long("LastGossipShare", last_share);
        }
        let last_worked = self.last_worked_at_poi.load(Ordering::Relaxed);
        if last_worked > 0 {
            nbt.put_long("LastWorkedAtPoi", last_worked);
        }
        if let Some(pos) = job_site {
            nbt.put_int("JobSiteX", pos.0.x);
            nbt.put_int("JobSiteY", pos.0.y);
            nbt.put_int("JobSiteZ", pos.0.z);
            nbt.put_bool(
                "JobSitePending",
                self.job_site_pending.load(Ordering::Relaxed),
            );
        }
        if let Some(pos) = home {
            nbt.put_int("HomeX", pos.0.x);
            nbt.put_int("HomeY", pos.0.y);
            nbt.put_int("HomeZ", pos.0.z);
        }
        write_offers(nbt, &offers);
        inventory.write_nbt(nbt);
        Self::write_gossips_nbt(nbt, &gossips);
    }
}

fn write_offers(nbt: &mut NbtCompound, offers: &[MerchantOffer]) {
    if offers.is_empty() {
        return;
    }
    let mut recipes = Vec::new();
    for offer in offers {
        let mut recipe = NbtCompound::new();
        let mut buy = NbtCompound::new();
        let mut sell = NbtCompound::new();
        let item_stack: &ItemStack = offer.base_cost_a.0.as_ref();
        item_stack.write_item_stack(&mut buy);
        recipe.put_compound("buy", buy);
        let item_stack: &ItemStack = offer.output.0.as_ref();
        item_stack.write_item_stack(&mut sell);
        recipe.put_compound("sell", sell);
        if let Some(cost_b) = &offer.cost_b {
            let mut buy_b = NbtCompound::new();
            let item_stack: &ItemStack = cost_b.0.as_ref();
            item_stack.write_item_stack(&mut buy_b);
            recipe.put_compound("buyB", buy_b);
        }
        recipe.put_int("uses", offer.uses);
        recipe.put_int("maxUses", offer.max_uses);
        recipe.put_bool("rewardExp", offer.reward_exp);
        recipe.put_int("xp", offer.xp);
        recipe.put_float("priceMultiplier", offer.price_multiplier);
        recipe.put_int("specialPrice", offer.special_price);
        recipe.put_int("demand", offer.demand);
        recipes.push(NbtTag::Compound(recipe));
    }
    let mut encoded = NbtCompound::new();
    encoded.put_list("Recipes", recipes);
    nbt.put_compound("Offers", encoded);
}
