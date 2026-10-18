#[allow(clippy::wildcard_imports)]
use super::*;
use crate::block::entities::beacon::BeaconBlockEntity;
use pumpkin_inventory::beacon_screen_handler::BeaconScreenHandler;
use pumpkin_protocol::java::server::play::SSetBeacon;

enum UpdateResult {
    Applied,
    NoPayment,
    InvalidEffects,
}

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

            if beacon_handler.has_valid_payment() {
                let levels = beacon_handler
                    .beacon_data
                    .get_property(BeaconBlockEntity::DATA_LEVELS as i32);
                let primary_id = packet.primary_effect.map(|v| v.0);
                let secondary_id = packet.secondary_effect.map(|v| v.0);

                if !BeaconBlockEntity::validate_effects(primary_id, secondary_id, levels) {
                    UpdateResult::InvalidEffects
                } else if !beacon_handler.consume_payment() {
                    UpdateResult::NoPayment
                } else {
                    beacon_handler.beacon_data.set_property(
                        BeaconBlockEntity::DATA_PRIMARY as i32,
                        BeaconBlockEntity::encode_effect_id(primary_id),
                    );
                    beacon_handler.beacon_data.set_property(
                        BeaconBlockEntity::DATA_SECONDARY as i32,
                        BeaconBlockEntity::encode_effect_id(secondary_id),
                    );
                    screen_handler.sync_state();

                    info!(
                        "Player {} updated beacon effects: primary {:?}, secondary {:?}",
                        player.gameprofile.name, primary_id, secondary_id
                    );
                    UpdateResult::Applied
                }
            } else {
                UpdateResult::NoPayment
            }
        };

        if matches!(result, UpdateResult::InvalidEffects) {
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
