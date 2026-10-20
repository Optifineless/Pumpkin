use std::sync::Arc;

use pumpkin_inventory::{
    beacon_screen_handler::BeaconScreenHandler,
    inventory::slot_transaction::{lock_owners, ordered_owners},
};

use crate::{
    block::entities::{BlockEntity, beacon::BeaconBlockEntity},
    entity::player::Player,
    world::World,
};

pub enum UpdateResult {
    Applied(Option<Publication>),
    NoPayment,
    InvalidEffects,
}

pub struct Publication {
    world: Arc<World>,
    entity: Arc<dyn BlockEntity>,
    play_sound: bool,
}

impl Publication {
    pub(crate) fn publish(self) {
        if !self
            .world
            .get_block_entity(&self.entity.get_position())
            .is_some_and(|live| Arc::ptr_eq(&live, &self.entity))
        {
            return;
        }
        self.world.update_block_entity(&self.entity);
        // dataAccess.set(1) sounds for paid reselection too, provided a beam was active.
        if self.play_sound {
            self.world.play_block_sound(
                pumpkin_data::sound::Sound::BlockBeaconPowerSelect,
                pumpkin_data::sound::SoundCategory::Blocks,
                self.entity.get_position(),
            );
        }
    }

    pub(crate) fn queue(self) {
        let publish = move || self.publish();
        if let Err(publish) = crate::world::guard_effects::defer(publish) {
            publish();
        }
    }
}

/// `BeaconMenu.updateEffects`: validate and pay once before publishing outside menu ownership.
pub fn update_effects(
    handler: &BeaconScreenHandler,
    player: &Arc<Player>,
    primary: Option<i32>,
    secondary: Option<i32>,
) -> UpdateResult {
    let owners = ordered_owners(&[handler.inventory.as_ref()]);
    let guards = lock_owners(&owners);
    if owners.iter().any(|owner| !owner.is_live()) || !handler.has_valid_payment() {
        return UpdateResult::NoPayment;
    }
    let levels = handler
        .beacon_data
        .get_property(BeaconBlockEntity::DATA_LEVELS as i32);
    if !BeaconBlockEntity::validate_effects(primary, secondary, levels) {
        return UpdateResult::InvalidEffects;
    }
    if !handler.consume_payment() {
        return UpdateResult::NoPayment;
    }
    let beacon = handler
        .block_entity
        .as_ref()
        .and_then(|entity| entity.clone().downcast::<BeaconBlockEntity>().ok());
    let publication = beacon.map_or_else(
        || {
            handler.beacon_data.set_property(
                BeaconBlockEntity::DATA_PRIMARY as i32,
                BeaconBlockEntity::encode_effect_id(primary),
            );
            handler.beacon_data.set_property(
                BeaconBlockEntity::DATA_SECONDARY as i32,
                BeaconBlockEntity::encode_effect_id(secondary),
            );
            None
        },
        |beacon| {
            let play_sound = beacon.has_active_beam();
            beacon.set_effects(primary, secondary);
            Some(Publication {
                world: player.world(),
                entity: beacon as Arc<dyn BlockEntity>,
                play_sound,
            })
        },
    );
    drop(guards);
    UpdateResult::Applied(publication)
}
