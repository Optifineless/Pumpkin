use std::sync::{Arc, Weak};

use pumpkin_inventory::screen_handler::{ScreenHandler, ScreenMutex, SharedScreenHandler};
use pumpkin_protocol::bedrock::{
    network_item::{ContainerName, FullContainerName},
    server::item_stack_request::ItemStackRequestSlotInfo,
};

use super::{
    inventory_request_beacon::BEDROCK_PAYMENT_SLOT,
    inventory_request_loom::{BEDROCK_DYE_SLOT, BEDROCK_INPUT_SLOT},
    item_stack_request::SlotUpdate,
};
use crate::entity::player::Player;

const RETAINED_REQUESTS: usize = 64;

#[derive(Clone)]
struct RequestResult {
    id: i32,
    slots: Vec<(FullContainerName, u8, i32)>,
}

#[derive(Clone, Default)]
pub struct Results {
    player: Weak<Player>,
    screen: Option<Weak<ScreenMutex<dyn ScreenHandler>>>,
    requests: Vec<RequestResult>,
    last_loom_sound_tick: i64,
}

impl Results {
    pub(super) fn bind(&mut self, player: &Arc<Player>, screen: &SharedScreenHandler) {
        if !self.player.ptr_eq(&Arc::downgrade(player))
            || !self
                .screen
                .as_ref()
                .is_some_and(|old| old.ptr_eq(&Arc::downgrade(screen)))
        {
            *self = Self {
                player: Arc::downgrade(player),
                screen: Some(Arc::downgrade(screen)),
                ..Self::default()
            };
        }
    }

    pub(super) fn forget(&mut self, id: i32) {
        self.requests.retain(|result| result.id != id);
    }

    pub(super) fn record(&mut self, id: i32, updates: &[SlotUpdate]) {
        if id >= 0 {
            return;
        }
        let slots = updates
            .iter()
            .filter(|update| {
                update.container_name.dynamic_id.is_none()
                    && update.count > 0
                    && update.stack_id.0 > 0
                    && matches!(
                        (update.container_name.container_name, update.slot_id),
                        (ContainerName::BeaconPayment, BEDROCK_PAYMENT_SLOT)
                            | (ContainerName::LoomInput, BEDROCK_INPUT_SLOT)
                            | (ContainerName::LoomDye, BEDROCK_DYE_SLOT)
                    )
            })
            .map(|update| {
                (
                    update.container_name.clone(),
                    update.slot_id,
                    update.stack_id.0,
                )
            })
            .collect::<Vec<_>>();
        if slots.is_empty() {
            return;
        }
        if self.requests.len() == RETAINED_REQUESTS {
            self.requests.remove(0);
        }
        self.requests.push(RequestResult { id, slots });
    }

    pub(super) fn resolve(&self, source: &ItemStackRequestSlotInfo) -> Option<i32> {
        if source.container_name.dynamic_id.is_some() {
            return None;
        }
        if source.stack_id >= 0 {
            return Some(source.stack_id);
        }
        // Dragonfly resolveID: a prediction names a prior authoritative result
        // at this exact address, and still has to match the live positive UID.
        self.requests
            .iter()
            .find(|result| result.id == source.stack_id)?
            .slots
            .iter()
            .find_map(|(name, slot, uid)| {
                (name == &source.container_name && *slot == source.slot_id).then_some(*uid)
            })
    }

    pub(super) fn queue_loom_sound(&mut self, player: &Arc<Player>) {
        let world = player.world();
        let tick = world.level.get_game_time();
        let Some(position) = player.open_container_pos.load() else {
            return;
        };
        if self.last_loom_sound_tick == tick {
            return;
        }
        // LoomMenu.resultSlot.onTake throttles the committed sound by game tick.
        self.last_loom_sound_tick = tick;
        let sound = move || {
            world.play_block_sound(
                pumpkin_data::sound::Sound::UiLoomTakeResult,
                pumpkin_data::sound::SoundCategory::Blocks,
                position,
            );
        };
        if let Err(sound) = crate::world::guard_effects::defer(sound) {
            sound();
        }
    }
}
