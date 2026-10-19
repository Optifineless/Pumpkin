use super::Player;
use pumpkin_protocol::{
    VarInt,
    java::client::play::{CPlayerInfoUpdate, PlayerAction, PlayerInfoFlags},
};
use pumpkin_util::text::TextComponent;
use std::sync::atomic::Ordering;

impl Player {
    pub(crate) fn broadcast_tab_update(&self, packet: &CPlayerInfoUpdate<'_>) {
        if let Some(server) = self.world().server.upgrade() {
            server.broadcast_player_info(self, packet);
        }
    }

    pub(crate) fn player_list_display_name(&self) -> Option<TextComponent> {
        self.get_tab_list_name().or_else(|| {
            self.display_name
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        })
    }

    pub fn set_display_name(&self, display_name: Option<TextComponent>) {
        let mut guard = self
            .display_name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = display_name;
        drop(guard);
        let name = self.player_list_display_name();
        // Update the tab list for everyone
        self.broadcast_tab_update(&CPlayerInfoUpdate::new(
            PlayerInfoFlags::UPDATE_DISPLAY_NAME.bits(),
            &[pumpkin_protocol::java::client::play::Player {
                uuid: self.gameprofile.id,
                actions: &[PlayerAction::UpdateDisplayName(name.as_ref())],
            }],
        ));
    }

    pub fn get_tab_list_name(&self) -> Option<TextComponent> {
        self.tab_list_name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_tab_list_name(&self, name: Option<TextComponent>) {
        let mut guard = self
            .tab_list_name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = name;
        drop(guard);
        let name = self.player_list_display_name();
        self.broadcast_tab_update(&CPlayerInfoUpdate::new(
            PlayerInfoFlags::UPDATE_DISPLAY_NAME.bits(),
            &[pumpkin_protocol::java::client::play::Player {
                uuid: self.gameprofile.id,
                actions: &[PlayerAction::UpdateDisplayName(name.as_ref())],
            }],
        ));
    }

    pub fn set_tab_list_order(&self, order: i32) {
        self.tab_list_order.store(order, Ordering::Relaxed);
        self.broadcast_tab_update(&CPlayerInfoUpdate::new(
            PlayerInfoFlags::UPDATE_LIST_PRIORITY.bits(),
            &[pumpkin_protocol::java::client::play::Player {
                uuid: self.gameprofile.id,
                actions: &[PlayerAction::UpdateListOrder(VarInt(order))],
            }],
        ));
    }

    pub fn set_tab_list_latency(&self, latency: i32) {
        self.tab_list_latency.store(latency, Ordering::Relaxed);
        self.broadcast_tab_update(&CPlayerInfoUpdate::new(
            PlayerInfoFlags::UPDATE_LATENCY.bits(),
            &[pumpkin_protocol::java::client::play::Player {
                uuid: self.gameprofile.id,
                actions: &[PlayerAction::UpdateLatency(VarInt(latency))],
            }],
        ));
    }

    pub fn set_tab_list_listed(&self, listed: bool) {
        self.tab_list_listed.store(listed, Ordering::Relaxed);
        self.broadcast_tab_update(&CPlayerInfoUpdate::new(
            PlayerInfoFlags::UPDATE_LISTED.bits(),
            &[pumpkin_protocol::java::client::play::Player {
                uuid: self.gameprofile.id,
                actions: &[PlayerAction::UpdateListed(listed)],
            }],
        ));
    }
}
