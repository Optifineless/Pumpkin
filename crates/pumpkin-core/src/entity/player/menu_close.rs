use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use pumpkin_inventory::screen_handler::{ScreenHandler, ScreenMutex};

use super::Player;

type Menu = Arc<ScreenMutex<dyn ScreenHandler>>;

/// A plugin can reopen a menu while its predecessor's close event is running.
#[derive(Default)]
pub(super) struct MenuCloseState(Mutex<HashSet<usize>>);

pub(super) struct ClosingMenu<'a> {
    state: &'a MenuCloseState,
    pub(super) menu: Menu,
}

fn menu_key(menu: &Menu) -> usize {
    Arc::as_ptr(menu).cast::<()>() as usize
}

impl MenuCloseState {
    fn claim(&self, menu: Menu) -> Option<ClosingMenu<'_>> {
        let mut active = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !active.insert(menu_key(&menu)) {
            return None;
        }
        Some(ClosingMenu { state: self, menu })
    }
}

impl Drop for ClosingMenu<'_> {
    fn drop(&mut self) {
        self.state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&menu_key(&self.menu));
    }
}

impl Player {
    pub(super) fn begin_menu_close(&self) -> Option<ClosingMenu<'_>> {
        let menu = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        self.menu_close_state.claim(menu)
    }

    pub fn on_handled_screen_closed(&self) {
        let _effects = crate::world::guard_effects::GuardEffects::new();
        let Some(closing) = self.begin_menu_close() else {
            return;
        };
        self.finish_handled_screen_closed(&closing.menu);
    }

    pub(super) fn closing_menu_is_selected(&self, closing: &Menu) -> bool {
        let selected = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::ptr_eq(&selected, closing)
    }

    pub(super) fn publish_closed_menu(&self, closing: &Menu) {
        // ServerPlayer.doCloseContainer publishes inventoryMenu. Pumpkin's close
        // event can install a replacement, whose selection and position it must retain.
        let mut selected = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Arc::ptr_eq(&selected, closing) {
            *selected = self.player_screen_handler.clone();
            self.open_container_pos.store(None);
        }
    }
}
