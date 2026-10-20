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

pub(super) const BEDROCK_PAYMENT_SLOT: u8 = 27;

// Bedrock zero is absent; Java registry zero is Speed.
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
    predictions: &super::inventory_request_predictions::Results,
) -> bool {
    // The ordinary payment has already entered the menu in its own move request.
    // Admit the entire shape before the operation can debit that payment.
    let Some((
        Action::BeaconPayment {
            primary_effect_id,
            secondary_effect_id,
        },
        suffix,
    )) = actions.split_first()
    else {
        return false;
    };
    let Some(beacon) = handler.as_any_mut().downcast_mut::<BeaconScreenHandler>() else {
        return false;
    };
    // Dragonfly handleBeaconPayment documents the client's predicted Destroy.
    // Validate it before Java's updateEffects performs the only debit.
    let payment = beacon
        .inventory
        .get_stack(BeaconScreenHandler::PAYMENT_SLOT);
    match suffix {
        [] => {}
        [Action::Destroy { count: 1, source }]
            if source.container_name.container_name == ContainerName::BeaconPayment
                && source.container_name.dynamic_id.is_none()
                && source.slot_id == BEDROCK_PAYMENT_SLOT
                && predictions.resolve(source) == Some(payment.uid.get()) => {}
        _ => return false,
    }
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
        BEDROCK_PAYMENT_SLOT,
        &payment,
    );
    if let Some(publication) = publication {
        publication.queue();
    }
    true
}
