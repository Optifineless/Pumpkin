use pumpkin_inventory::{
    enchanting::enchanting_screen_handler::EnchantingTableScreenHandler,
    screen_handler::{InventoryPlayer, ScreenHandler, ScreenMutex, SharedScreenHandler},
};
use pumpkin_protocol::java::server::play::SContainerButtonClick;
use std::sync::Arc;

use super::Player;
use crate::entity::EntityBase;
use crate::world::guard_effects::{GuardEffects, defer};

impl Player {
    /// Sends captured equipment changes after the current menu operation unlocks.
    pub(crate) fn send_menu_equipment(
        &self,
        equipment: &[(
            pumpkin_data::data_component_impl::EquipmentSlot,
            pumpkin_data::item_stack::ItemStack,
        )],
    ) {
        // ServerGamePacketListenerImpl.handleContainerClick completes the slot change first.
        let captured = equipment.to_vec();
        if !self
            .defer_menu_effect(move |player| player.living_entity.send_equipment_changes(&captured))
        {
            self.living_entity.send_equipment_changes(equipment);
        }
    }

    pub(super) fn dispatch_menu_callback(callback: Box<dyn FnOnce() + Send>) {
        if let Err(callback) = defer(callback) {
            callback();
        }
    }

    pub(super) fn notify_experience_level(&self, old_level: i32, level: i32) {
        if old_level == level {
            return;
        }
        // Player.onEnchantmentPerformed commits XP before completing the menu operation.
        // The non-cancellable level notification must leave that menu's guard first.
        if self.defer_menu_effect(move |player| player.notify_experience_level(old_level, level)) {
            return;
        }
        if let Some(server) = self.world().server.upgrade()
            && let Some(player) = self.world().get_player_by_uuid(self.gameprofile.id)
            && std::ptr::eq(self, player.as_ref())
        {
            let mut event = crate::plugin::api::events::player::player_level_change::PlayerLevelChangeEvent::new(
                player, old_level, level,
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    pub(super) fn drop_menu_item(&self, item: pumpkin_data::item_stack::ItemStack) {
        // AbstractContainerMenu.dropOrPlaceInInventory decides removal before returning items.
        if self.get_entity().is_removed() {
            let world = self.world();
            let pos = self.position().to_block_pos();
            let effect = move || world.drop_stack(&pos, item);
            if let Err(effect) = defer(effect) {
                effect();
            }
        } else {
            let copy = item.clone();
            if !self.defer_menu_effect(move |player| player.drop_item(copy)) {
                self.drop_item(item);
            }
        }
    }
    /// Publishes menu slot changes after releasing the menu guard.
    pub(crate) fn send_menu_updates<T: ScreenHandler + ?Sized>(screen: &ScreenMutex<T>) {
        // AbstractContainerMenu.broadcastChanges captures changes before notifying listeners.
        let _effects = GuardEffects::new();
        screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send_content_updates();
    }

    /// Removes the inventory menu while keeping its item returns outside the menu guard.
    pub(crate) fn remove_inventory_menu(&self) {
        // AbstractContainerMenu.removed returns carried and temporary crafting items.
        let _effects = GuardEffects::new();
        self.player_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .on_closed(self);
    }
    pub(super) fn play_menu_block_sound(&self, sound: pumpkin_data::sound::Sound, pitch: f32) {
        // EnchantmentMenu.clickMenuButton captures the table position through its access.
        let Some(pos) = self.open_container_pos.load() else {
            return;
        };
        let world = self.world();
        let effect = move || {
            world.play_sound_fine(
                sound,
                pumpkin_data::sound::SoundCategory::Blocks,
                &pos.to_centered_f64(),
                1.0,
                pitch,
            );
        };
        if let Err(effect) = defer(effect) {
            effect();
        }
    }
    pub(super) fn send_menu_packet(&self, packet: &impl pumpkin_protocol::ClientPacket) {
        if let Some(client) = self.client.java()
            && let Ok(bytes) = client.serialize_packet(packet)
        {
            crate::world::guard_effects::send(self.client.clone(), bytes);
        }
    }
    pub(super) fn click_menu_button(&self, packet: &SContainerButtonClick) {
        let screen = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let proposal = {
            let _effects = GuardEffects::new();
            let mut guard = screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if i32::from(guard.sync_id()) != packet.window_id.0 {
                return;
            }
            if let Some(menu) = guard
                .as_any_mut()
                .downcast_mut::<EnchantingTableScreenHandler>()
            {
                menu.prepare_enchantment(self, packet.button_id.0)
            } else {
                guard.on_button_click(self, packet.button_id.0);
                return;
            }
        };
        let Some(mut proposal) = proposal else {
            return;
        };
        if self.fire_enchant_item_event(
            &proposal.item,
            proposal.option,
            proposal.level_requirement,
            &mut proposal.enchantments,
        ) {
            return;
        }
        {
            let _effects = GuardEffects::new();
            if !self.menu_player_is_current() {
                return;
            }
            // Keep menu identity stable during commit; the callback ran before these guards.
            let current = self
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !Arc::ptr_eq(&screen, &current) {
                return;
            }
            let mut guard = screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(menu) = guard
                .as_any_mut()
                .downcast_mut::<EnchantingTableScreenHandler>()
            {
                menu.commit_enchantment(self, proposal);
            }
        }
        self.finish_menu_preparation(&screen);
    }

    pub(super) fn finish_menu_preparation(&self, screen: &SharedScreenHandler) {
        if !self.menu_player_is_current() {
            return;
        }
        let offers = {
            let current = self
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !Arc::ptr_eq(screen, &current) || self.get_entity().is_removed() {
                return;
            }
            let mut guard = screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard
                .as_any_mut()
                .downcast_mut::<EnchantingTableScreenHandler>()
                .and_then(EnchantingTableScreenHandler::take_prepare_offers)
        };
        let Some(mut offers) = offers else {
            return;
        };
        let cancelled = self.fire_prepare_item_enchant_event(
            &offers.item,
            &mut offers.costs,
            &mut offers.ids,
            &mut offers.levels,
            offers.bookshelves,
        );
        if !self.menu_player_is_current() {
            return;
        }
        let _effects = GuardEffects::new();
        let current = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Arc::ptr_eq(screen, &current) || self.get_entity().is_removed() {
            return;
        }
        let mut guard = screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(menu) = guard
            .as_any_mut()
            .downcast_mut::<EnchantingTableScreenHandler>()
        {
            menu.apply_prepare_offers(&offers, cancelled);
        }
    }

    /// Queues a non-cancellable menu effect for the same player instance after unlocking.
    pub(super) fn defer_menu_effect(&self, effect: impl FnOnce(&Self) + Send + 'static) -> bool {
        if !crate::world::guard_effects::is_active() {
            return false;
        }
        let Some(player) = self.world().get_player_by_uuid(self.gameprofile.id) else {
            return false;
        };
        if !std::ptr::eq(self, player.as_ref()) {
            return false;
        }
        defer(move || effect(&player)).is_ok()
    }

    pub(super) fn menu_player_is_current(&self) -> bool {
        // A plugin callback can replace the live session without changing this old menu.
        self.world()
            .get_player_by_uuid(self.gameprofile.id)
            .is_some_and(|player| std::ptr::eq(self, player.as_ref()))
    }
}
