use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const PHASE_BITS: u32 = 8;

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
// MinecraftServer.tickChildren / ServerLevel.tick profiler boundaries, plus Pumpkin workers.
pub enum Phase {
    ServerTick = 1,
    TickGate,
    TickStartEvent,
    FunctionsAndPlugins,
    WorldTick,
    Environment,
    ChunkTicks,
    ScheduledBlocks,
    ScheduledFluids,
    RandomTicks,
    BlockEvents,
    EntityPreparation,
    EntityTicks,
    PlayerTicks,
    Packets,
    BlockEntityPreparation,
    BlockEntityTicks,
    EntityTracking,
    ChunkNotifications,
    DragonFight,
    Autosave,
    BlockUpdates,
    GameTests,
    TickEndEvent,
    TickStatistics,
}

impl Phase {
    pub(super) const fn name(id: u8) -> &'static str {
        match id {
            1 => "server tick start",
            2 => "waiting for tick gate",
            3 => "tick start event",
            4 => "datapack functions / plugins",
            5 => "world tick",
            6 => "environment / raids",
            7 => "chunk ticks",
            8 => "scheduled block ticks",
            9 => "scheduled fluid ticks",
            10 => "random ticks / spawning",
            11 => "block events",
            12 => "entity preparation",
            13 => "entity ticks / player collisions",
            14 => "player ticks",
            15 => "packet processing",
            16 => "block entity preparation",
            17 => "block entity ticks / comparator updates",
            18 => "entity tracking",
            19 => "chunk notifications",
            20 => "dragon fight",
            21 => "autosave",
            22 => "block updates / network flush",
            23 => "game tests",
            24 => "tick end event",
            25 => "tick statistics",
            _ => "unknown",
        }
    }

    /// Updates the active tick subject on this worker, without touching game state.
    pub fn enter(self) {
        super::slots::enter_phase(self);
    }
}

#[derive(Default)]
pub struct Record {
    // Low byte: phase; upper 56 bits: monotonic milliseconds. Zero means inactive.
    value: AtomicU64,
}

impl Record {
    pub const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    pub fn enter(&self, phase: Phase, now_ms: u64) {
        self.value
            .store((now_ms << PHASE_BITS) | phase as u64, Ordering::Relaxed);
    }

    pub(super) fn finish(&self) {
        self.value.store(0, Ordering::Relaxed);
    }

    pub fn describe(&self, now_ms: u64) -> Option<PhaseDescription> {
        let value = self.value.load(Ordering::Relaxed);
        if value == 0 {
            return None;
        }
        let phase = Phase::name(value as u8);
        let age = now_ms.saturating_sub(value >> PHASE_BITS);
        Some(PhaseDescription { phase, age })
    }
}

/// Formats a copied phase observation without allocating a string.
pub struct PhaseDescription {
    phase: &'static str,
    age: u64,
}

impl fmt::Display for PhaseDescription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}, entered {} ms ago", self.phase, self.age)
    }
}

pub fn elapsed_ms(epoch: Instant) -> u64 {
    epoch.elapsed().as_millis() as u64
}
