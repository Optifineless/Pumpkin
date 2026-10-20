#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_inventory::beacon_screen_handler::BeaconScreenHandler;
use pumpkin_protocol::java::server::play::SSetBeacon;

use super::beacon_update::{UpdateResult, update_effects};

impl JavaClient {
    pub fn handle_set_beacon(&self, player: &Arc<Player>, packet: &SSetBeacon) {
        // BeaconMenu.updateEffects commits payment before broadcasting the menu state.
        let _effects = crate::world::guard_effects::GuardEffects::new();
        let result = {
            let screen_handler_lock = player
                .current_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let mut screen_handler = screen_handler_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            let Some(beacon_handler) = screen_handler
                .as_any_mut()
                .downcast_mut::<BeaconScreenHandler>()
            else {
                debug!(
                    "Player {} interacted with invalid menu (expected beacon)",
                    player.gameprofile.name
                );
                return;
            };

            // ServerGamePacketListenerImpl.handleSetBeaconPacket re-checks stillValid before
            // updateEffects; the next player tick closes the menu and returns the payment.
            if !beacon_handler.can_use(player.as_ref()) {
                return;
            }

            let result = update_effects(
                beacon_handler,
                player,
                packet.primary_effect.map(|value| value.0),
                packet.secondary_effect.map(|value| value.0),
            );
            if matches!(result, UpdateResult::Applied(_)) {
                screen_handler.sync_state();
                info!(
                    "Player {} updated beacon effects: primary {:?}, secondary {:?}",
                    player.gameprofile.name,
                    packet.primary_effect.map(|value| value.0),
                    packet.secondary_effect.map(|value| value.0)
                );
            }
            result
        };

        // Preserve Java's saved-image capture before queued menu notifications flush.
        if let UpdateResult::Applied(Some(publication)) = result {
            publication.publish();
        } else if matches!(result, UpdateResult::InvalidEffects) {
            let primary_id = packet.primary_effect.map(|v| v.0);
            let secondary_id = packet.secondary_effect.map(|v| v.0);
            warn!(
                "Player {} tried to set invalid beacon effects: primary {:?}, secondary {:?}",
                player.gameprofile.name, primary_id, secondary_id
            );
            self.try_kick(&TextComponent::translate(
                "multiplayer.disconnect.generic",
                &[],
            ));
        }
    }
}
