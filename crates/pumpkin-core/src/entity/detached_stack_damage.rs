use super::player::Player;

impl Player {
    /// Fires [`PlayerItemDamageEvent`], and [`PlayerItemBreakEvent`] when `broken`, for an item that took damage.
    pub fn fire_item_damage_events(
        &self,
        item: &pumpkin_data::item::Item,
        amount: i32,
        broken: bool,
    ) {
        let world = self.world();
        let Some(server) = world.server.upgrade() else {
            return;
        };
        let Some(player_arc) = world.get_player_by_uuid(self.gameprofile.id) else {
            return;
        };

        let mut event =
            crate::plugin::api::events::player::player_item_damage::PlayerItemDamageEvent::new(
                player_arc.clone(),
                item.registry_key.to_string(),
                amount,
            );
        server.plugin_manager.fire_blocking(&server, &mut event);
        if broken {
            let mut event =
                crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent::new(
                    player_arc,
                    item.registry_key.to_string(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }
}

use crate::enchantment::helper::EnchantmentHelper;
use pumpkin_data::item_stack::{DamageResult, ItemStack};
use pumpkin_util::GameMode;

impl Player {
    /// Damages an interaction clone before its authoritative hand write-back.
    pub(crate) fn damage_detached_item(&self, stack: &mut ItemStack, amount: i32) {
        // ItemStack.hurtAndBreak / processDurabilityChange; callbacks precede mutation.
        if self.gamemode.load() == GameMode::Creative
            || stack.is_empty()
            || !stack.is_damageable()
            || stack.is_unbreakable()
        {
            return;
        }
        let mut amount = EnchantmentHelper::modify_durability_change(stack, amount as f32) as i32;
        if amount <= 0 {
            return;
        }
        let world = self.world();
        let server = world.server.upgrade();
        let player = world.get_player_by_uuid(self.gameprofile.id);
        if let (Some(server), Some(player)) = (&server, &player) {
            let mut event =
                crate::plugin::api::events::player::player_item_damage::PlayerItemDamageEvent::new(
                    player.clone(),
                    stack.item.registry_key.to_string(),
                    amount,
                );
            server.plugin_manager.fire_blocking(server, &mut event);
            if event.cancelled {
                return;
            }
            amount = event.damage;
        }
        if amount <= 0 {
            return;
        }
        let item = stack.item;
        if stack.damage_item(amount) == DamageResult::Broken
            && let (Some(server), Some(player)) = (server, player)
        {
            let mut event =
                crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent::new(
                    player,
                    item.registry_key.to_string(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        // The shared hand write-back sends exactly one break status/stat before slot sync.
    }
}
