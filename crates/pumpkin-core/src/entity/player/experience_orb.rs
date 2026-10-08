use super::Player;
use crate::{enchantment::EnchantmentHelper, entity::equipment_damage::with_slot};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_protocol::java::client::play::CSetPlayerInventory;
use std::sync::atomic::Ordering;

impl Player {
    /// `Player.aiStep` only collects while alive and outside spectator mode.
    #[must_use]
    pub fn can_collect_experience(&self) -> bool {
        !self.is_spectator()
            && !self.living_entity.dead.load(Ordering::Relaxed)
            && self.living_entity.health.load() > 0.0
            && !self.living_entity.entity.is_removed()
    }

    /// Admits one orb pickup and starts `Player.takeXpDelay`'s two-tick cooldown.
    pub fn try_take_experience(&self) -> bool {
        self.experience_pick_up_delay
            .compare_exchange(0, 2, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    /// Decrements `Player.takeXpDelay` once per player tick, without lock contention.
    pub fn tick_experience_pickup_delay(&self) {
        self.experience_pick_up_delay
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |delay| {
                (delay > 0).then(|| delay - 1)
            })
            .ok();
    }

    /// Repairs equipped items, spending XP with `ExperienceOrb.repairPlayerItems`' integer division.
    pub fn apply_mending_from_xp(&self, mut amount: i32) -> i32 {
        // Vanilla recurses only after a positive repair; iteration avoids a deep server stack.
        while amount > 0 {
            let Some(item) = EnchantmentHelper::get_random_item_with_repair_effect(self) else {
                return amount;
            };
            let Some(index) = item.inventory_index else {
                return amount;
            };
            let capacity =
                EnchantmentHelper::modify_durability_to_repair_from_xp(&item.stack, amount);
            let repair = capacity.min(item.stack.get_damage());
            if repair <= 0 {
                return 0;
            }
            let spent = (i64::from(repair) * i64::from(amount) / i64::from(capacity)) as i32;
            let world = self.world();
            if let Some(player) = world.get_player_by_uuid(self.gameprofile.id)
                && let Some(server) = world.server.upgrade()
            {
                let mut event =
                    crate::plugin::api::events::player::player_item_mend::PlayerItemMendEvent {
                        player,
                        item_name: item.stack.item.registry_key.to_string(),
                        repair_amount: repair,
                        exp_consumed: spent,
                        cancelled: false,
                    };
                server.plugin_manager.fire_blocking(&server, &mut event);
                if event.cancelled {
                    return amount;
                }
            }
            let updated = with_slot(&self.inventory, index, |stack| {
                // Plugins and packet handlers can replace a slot while the callback runs.
                if stack.uid != item.stack.uid
                    || !ItemStack::are_items_and_components_equal(stack, &item.stack)
                {
                    return None;
                }
                stack.set_damage(stack.get_damage() - repair);
                Some(stack.clone())
            })
            .flatten();
            let Some(updated) = updated else {
                return amount;
            };
            self.try_send_slot_set_packet(&CSetPlayerInventory::new(
                (index as i32).into(),
                &ItemStackSerializer::from(updated.clone()),
            ));
            self.sync_inventory_to_client();
            self.living_entity
                .send_equipment_changes(&[(item.slot, updated)]);
            amount -= spent;
        }
        0
    }
}
