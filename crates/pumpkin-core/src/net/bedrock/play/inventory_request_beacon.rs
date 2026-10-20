use std::sync::Arc;

use pumpkin_data::effect::StatusEffect;
use pumpkin_inventory::{
    beacon_screen_handler::BeaconScreenHandler, screen_handler::ScreenHandler,
};
use pumpkin_protocol::bedrock::{
    network_item::{ContainerName, FullContainerName},
    server::item_stack_request::ItemStackRequestAction as Action,
};

use crate::{
    entity::player::Player,
    net::java::play::beacon_update::{UpdateResult, update_effects},
};

use super::item_stack_request::{SlotUpdate, record_update};

// Supported request fixture: Bedrock zero is absent; Java registry zero is Speed.
// The actual 1.26.51 client absent-marker capture remains unverified.
fn effect(id: i32) -> Result<Option<i32>, ()> {
    if id == 0 {
        return Ok(None);
    }
    [
        &StatusEffect::SPEED,
        &StatusEffect::HASTE,
        &StatusEffect::RESISTANCE,
        &StatusEffect::JUMP_BOOST,
        &StatusEffect::STRENGTH,
        &StatusEffect::REGENERATION,
    ]
    .into_iter()
    .find(|effect| effect.to_bedrock_id() == id)
    .map(|effect| Some(i32::from(effect.id)))
    .ok_or(())
}

pub(super) fn apply(
    handler: &mut dyn ScreenHandler,
    player: &Arc<Player>,
    actions: &[Action],
    updates: &mut Vec<SlotUpdate>,
) -> bool {
    // The ordinary payment has already entered the menu in its own move request.
    // Admit the entire shape before the operation can debit that payment.
    let [
        Action::BeaconPayment {
            primary_effect_id,
            secondary_effect_id,
        },
    ] = actions
    else {
        return false;
    };
    let Some(beacon) = handler.as_any_mut().downcast_mut::<BeaconScreenHandler>() else {
        return false;
    };
    let (Ok(primary), Ok(secondary)) = (effect(primary_effect_id.0), effect(secondary_effect_id.0))
    else {
        return false;
    };
    let UpdateResult::Applied(publication) = update_effects(beacon, player, primary, secondary)
    else {
        return false;
    };
    let payment = beacon
        .inventory
        .get_stack(BeaconScreenHandler::PAYMENT_SLOT);
    record_update(
        updates,
        FullContainerName {
            container_name: ContainerName::BeaconPayment,
            dynamic_id: None,
        },
        0,
        &payment,
    );
    if let Some(publication) = publication {
        publication.queue();
    }
    true
}
