#[allow(clippy::wildcard_imports)]
use super::*;
use crate::block::entities::{BlockEntity, beacon::BeaconBlockEntity};
use pumpkin_inventory::beacon_screen_handler::BeaconScreenHandler;
use pumpkin_protocol::java::server::play::SSetBeacon;

enum UpdateResult {
    Applied(Option<(Arc<World>, Arc<dyn BlockEntity>, bool)>),
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
                    let beacon = beacon_handler
                        .block_entity
                        .as_ref()
                        .and_then(|entity| entity.clone().downcast::<BeaconBlockEntity>().ok());
                    let updated = if let Some(beacon) = beacon {
                        let play_sound = beacon.has_active_beam();
                        beacon.set_effects(primary_id, secondary_id);
                        Some((player.world(), beacon as Arc<dyn BlockEntity>, play_sound))
                    } else {
                        beacon_handler.beacon_data.set_property(
                            BeaconBlockEntity::DATA_PRIMARY as i32,
                            BeaconBlockEntity::encode_effect_id(primary_id),
                        );
                        beacon_handler.beacon_data.set_property(
                            BeaconBlockEntity::DATA_SECONDARY as i32,
                            BeaconBlockEntity::encode_effect_id(secondary_id),
                        );
                        None
                    };
                    screen_handler.sync_state();

                    info!(
                        "Player {} updated beacon effects: primary {:?}, secondary {:?}",
                        player.gameprofile.name, primary_id, secondary_id
                    );
                    UpdateResult::Applied(updated)
                }
            } else {
                UpdateResult::NoPayment
            }
        };

        // BeaconMenu.updateEffects calls blockEntityChanged. Admission is below the
        // screen lock in the lock hierarchy, so refresh the saved image only after unlocking.
        if let UpdateResult::Applied(Some((world, entity, play_sound))) = &result
            && world
                .get_block_entity(&entity.get_position())
                .is_some_and(|live| Arc::ptr_eq(&live, entity))
        {
            world.update_block_entity(entity);
            // BeaconBlockEntity.dataAccess.set(1) sounds for every paid primary selection
            // with a beam, including reselecting the current power.
            if *play_sound {
                world.play_block_sound(
                    pumpkin_data::sound::Sound::BlockBeaconPowerSelect,
                    pumpkin_data::sound::SoundCategory::Blocks,
                    entity.get_position(),
                );
            }
        }

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
