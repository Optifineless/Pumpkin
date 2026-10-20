//! `AbstractHorse`'s committed fields share one owner, including native setters and NBT capture.
use super::horse::{FLAG_STANDING, FLAG_TAME, HorseEntity};
use crate::entity::EntityBase;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::sync::{LockClass, Mutex};
use uuid::Uuid;

pub(super) enum HorseStateLock {}
impl LockClass for HorseStateLock {
    const NAME: &'static str = "horse state";
    const LEVEL: u8 = 30;
}

#[derive(Default)]
pub(super) struct HorseState {
    pub flags: u8,
    pub temper: i32,
    pub owner: Option<Uuid>,
    pub revision: u64,
    pub riding_epoch: u64,
    pub next_attempt: u64,
    pub pending: Option<u64>,
    pub standing_ticks: u8,
    publishing_flags: bool,
}

pub(super) type State = Mutex<HorseState, HorseStateLock>;

impl HorseEntity {
    pub(crate) fn invalidate_ride_attempt(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.riding_epoch = state.riding_epoch.wrapping_add(1);
    }
    pub(super) fn clear_standing(&self) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.standing_ticks = 0;
            state.flags &= !FLAG_STANDING;
        };
        self.publish_flags();
    }
    // No horse state guard reaches navigation/selectors, relationships, plugins or packets.
    // All server/native writers (set_flag/tame/temper/owner, NBT and ride commits) use state;
    // inherent/trait readers and save capture use it too. Variant remains independent.
    #[must_use]
    pub fn has_flag(&self, flag: u8) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .flags
            & flag
            != 0
    }

    pub fn set_flag(&self, flag: u8, value: bool) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.get_entity().is_removed() {
                return;
            }
            state.flags = if value {
                state.flags | flag
            } else {
                state.flags & !flag
            };
            state.revision = state.revision.wrapping_add(1);
        };
        self.publish_flags();
    }

    #[must_use]
    pub fn get_temper(&self) -> i32 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .temper
    }

    /// Sets the saved value verbatim, like AbstractHorse.setTemper (gameplay increments clamp).
    pub fn set_temper(&self, temper: i32) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.get_entity().is_removed() {
            state.temper = temper;
            state.revision = state.revision.wrapping_add(1);
        }
    }

    #[must_use]
    pub const fn get_max_temper(&self) -> i32 {
        100
    }

    #[must_use]
    pub fn get_owner(&self) -> Option<Uuid> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner
    }

    pub fn set_owner(&self, owner: Option<Uuid>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.get_entity().is_removed() {
            state.owner = owner;
            state.revision = state.revision.wrapping_add(1);
        }
    }

    pub(super) fn write_horse_nbt(&self, nbt: &mut NbtCompound) {
        let (tame, temper, owner) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (state.flags & FLAG_TAME != 0, state.temper, state.owner)
        };
        nbt.put_bool("Tame", tame);
        nbt.put_int("Temper", temper);
        if let Some(owner) = owner {
            nbt.put_uuid("Owner", owner);
        }
    }

    pub(super) fn read_horse_nbt(&self, nbt: &NbtCompound) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.get_entity().is_removed() {
                return;
            }
            if let Some(tame) = nbt.get_bool("Tame") {
                state.flags = if tame {
                    state.flags | FLAG_TAME
                } else {
                    state.flags & !FLAG_TAME
                };
            }
            if let Some(temper) = nbt.get_int("Temper") {
                state.temper = temper;
            }
            if let Some(owner) = nbt.get_uuid("Owner") {
                state.owner = Some(owner);
            }
            state.revision = state.revision.wrapping_add(1);
        };
        self.publish_flags();
    }

    pub(super) fn tick_standing(&self) {
        let cleared = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.standing_ticks == 0 {
                return;
            }
            state.standing_ticks -= 1;
            if state.standing_ticks == 0 {
                state.flags &= !FLAG_STANDING;
                true
            } else {
                false
            }
        };
        if cleared {
            self.publish_flags();
        }
    }

    // A detached single publisher prevents an older snapshot overwriting newer packed flags.
    // set_synced_data only queues tracker metadata; it cannot call plugins or send packets.
    pub(super) fn publish_flags(&self) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.publishing_flags {
                return;
            }
            state.publishing_flags = true;
        };
        loop {
            let flags = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .flags;
            self.get_entity().set_synced_data(
                pumpkin_data::tracked_data::horse::DATA_ID_FLAGS,
                flags as i8,
            );
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.flags == flags {
                state.publishing_flags = false;
                return;
            }
        }
    }
}
