use std::sync::Arc;

use pumpkin_data::screen::WindowType;
use pumpkin_inventory::{
    beacon_screen_handler::BeaconScreenHandler,
    inventory::slot_transaction,
    screen_handler::{InventoryPlayer, ScreenHandler, SharedScreenHandler},
};
use pumpkin_protocol::bedrock::server::item_stack_request::{
    ItemStackRequest, ItemStackRequestAction as Action, SItemStackRequest,
};
use pumpkin_util::GameMode;

use super::item_stack_request::SlotUpdate;
use crate::{
    entity::{EntityBase, player::Player},
    world::World,
};

const DEFAULT_DISTANCE_BUFFER: f64 = 4.0;
type ValidityCheck = Box<dyn Fn(&dyn InventoryPlayer) -> bool + Send + Sync>;

pub(super) struct Admission {
    screen: SharedScreenHandler,
    usable: bool,
    world: Arc<World>,
    position: Option<pumpkin_util::math::position::BlockPos>,
}

pub(super) fn queue_sync(admission: &Admission, player: &Arc<Player>) {
    let screen = admission.screen.clone();
    let player = Arc::downgrade(player);
    let publish = move || {
        let Some(player) = player.upgrade() else {
            return;
        };
        if player.client.closed()
            || player.get_entity().is_removed()
            || !player
                .world()
                .get_player_by_uuid(player.gameprofile.id)
                .is_some_and(|live| Arc::ptr_eq(&live, &player))
        {
            return;
        }
        let _effects = crate::world::guard_effects::GuardEffects::new();
        let current = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Arc::ptr_eq(&current, &screen) {
            screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .send_content_updates();
        }
    };
    if let Err(publish) = crate::world::guard_effects::defer(publish) {
        publish();
    }
}

// Beacon validity is a native callback. Temporarily fail closed while running
// its existing check without menu/session guards, then restore only our marker.
pub(super) fn prepare(player: &Arc<Player>, packet: &SItemStackRequest) -> Option<Admission> {
    if !packet
        .requests
        .iter()
        .flat_map(|request| &request.actions)
        .any(|action| matches!(action, Action::BeaconPayment { .. } | Action::Loom { .. }))
    {
        return None;
    }
    let screen = player
        .current_screen_handler
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let world = player.world();
    let position = player.open_container_pos.load();
    let marker = std::sync::atomic::AtomicBool::new(false);
    let sentinel: ValidityCheck =
        Box::new(move |_| marker.load(std::sync::atomic::Ordering::Relaxed));
    let sentinel_ptr = std::ptr::from_ref(sentinel.as_ref()).cast::<()>();
    let validity = {
        let mut handler = screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(beacon) = handler.as_any_mut().downcast_mut::<BeaconScreenHandler>() {
            beacon.validity_check.replace(sentinel)
        } else {
            None
        }
    };
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        validity.as_ref().is_none_or(|check| check(player.as_ref()))
    }));
    let restored = {
        let mut handler = screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(beacon) = handler.as_any_mut().downcast_mut::<BeaconScreenHandler>() {
            if beacon.validity_check.as_ref().is_some_and(|check| {
                std::ptr::from_ref(check.as_ref()).cast::<()>() == sentinel_ptr
            }) {
                beacon.validity_check = validity;
                true
            } else {
                false
            }
        } else {
            true
        }
    };
    let usable = match checked {
        Ok(usable) => usable,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    Some(Admission {
        screen,
        usable: usable && restored,
        world,
        position,
    })
}

pub(super) fn apply(
    handler: &mut dyn ScreenHandler,
    screen: &SharedScreenHandler,
    player: &Arc<Player>,
    request: &ItemStackRequest,
    updates: &mut Vec<SlotUpdate>,
    admission: Option<&Admission>,
    predictions: &super::inventory_request_predictions::Results,
) -> Option<bool> {
    let specialized = request.actions.iter().any(|action| {
        matches!(action, Action::BeaconPayment { .. } | Action::Loom { .. })
            || (matches!(
                handler.window_type(),
                Some(WindowType::Beacon | WindowType::Loom)
            ) && matches!(
                action,
                Action::Grindstone { .. } | Action::LabTableCombine | Action::CraftNonImplemented
            ))
    });
    if !specialized {
        return None;
    }
    if player.gamemode.load() == GameMode::Spectator
        || player.client.closed()
        || player.get_entity().is_removed()
        || !player.living_entity.is_alive()
        || !player
            .world()
            .get_player_by_uuid(player.gameprofile.id)
            .is_some_and(|live| Arc::ptr_eq(player, &live))
        || !admission.is_some_and(|admission| {
            admission.usable
                && Arc::ptr_eq(&admission.screen, screen)
                && Arc::ptr_eq(&admission.world, &player.world())
                && admission.position == player.open_container_pos.load()
        })
    {
        return Some(false);
    }
    // The caller holds current-screen selection before taking this screen,
    // keeping the checked incarnation stable until the commit completes.
    if handler.window_type() == Some(WindowType::Loom)
        && !player.open_container_pos.load().is_some_and(|pos| {
            player.world().get_block(&pos) == &pumpkin_data::Block::LOOM
                && player.can_interact_with_block_at(&pos, DEFAULT_DISTANCE_BUFFER)
        })
    {
        return Some(false);
    }
    if handler.window_type() == Some(WindowType::Beacon) && !live_beacon(handler, player) {
        return Some(false);
    }

    let inventories = handler
        .get_behaviour()
        .slots
        .iter()
        .map(|slot| slot.get_inventory())
        .collect::<Vec<_>>();
    let refs = inventories.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    let owners = slot_transaction::ordered_owners(&refs);
    let _guards = slot_transaction::lock_owners(&owners);
    if owners.iter().any(|owner| !owner.is_live()) {
        return Some(false);
    }
    Some(match request.actions.first() {
        Some(Action::BeaconPayment { .. }) => super::inventory_request_beacon::apply(
            handler,
            player,
            &request.actions,
            updates,
            predictions,
        ),
        Some(Action::Loom { .. }) => {
            super::inventory_request_loom::apply(handler, player, request, updates, predictions)
        }
        _ => false,
    })
}

fn live_beacon(handler: &dyn ScreenHandler, player: &Player) -> bool {
    use crate::block::entities::{BlockEntity, beacon::BeaconBlockEntity};
    let beacon = handler
        .as_any()
        .downcast_ref::<BeaconScreenHandler>()
        .and_then(|handler| handler.block_entity.as_ref())
        .and_then(|entity| entity.clone().downcast::<BeaconBlockEntity>().ok());
    beacon.is_some_and(|beacon| {
        let position = beacon.get_position();
        let entity: Arc<dyn BlockEntity> = beacon;
        player.open_container_pos.load() == Some(position)
            && player.world().get_block(&position) == &pumpkin_data::Block::BEACON
            && player
                .world()
                .get_block_entity(&position)
                .is_some_and(|live| Arc::ptr_eq(&live, &entity))
            && player.can_interact_with_block_at(&position, DEFAULT_DISTANCE_BUFFER)
    })
}
