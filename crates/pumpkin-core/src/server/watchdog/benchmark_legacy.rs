// Pre-slot markers retained only for the ignored before/after benchmark.
use std::cell::RefCell;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

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
    const fn name(id: u8) -> &'static str {
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
        CURRENT.with_borrow(|current| {
            if let Some((group, index)) = current {
                group.records[*index].enter(self, elapsed_ms(group.epoch));
            }
        });
    }
}

pub enum Subject {
    World,
    Entity {
        id: i32,
        kind: &'static str,
        position: Vector3<f64>,
    },
    BlockEntity {
        kind: &'static str,
        position: BlockPos,
    },
}

pub struct Record {
    pub subject: Subject,
    // Low byte: phase; upper 56 bits: monotonic milliseconds. Zero means inactive.
    value: AtomicU64,
}

impl Record {
    pub const fn new(subject: Subject) -> Self {
        Self {
            subject,
            value: AtomicU64::new(0),
        }
    }

    pub fn enter(&self, phase: Phase, now_ms: u64) {
        self.value
            .store((now_ms << PHASE_BITS) | phase as u64, Ordering::Relaxed);
    }

    fn finish(&self) {
        self.value.store(0, Ordering::Relaxed);
    }

    pub fn describe(&self, now_ms: u64) -> Option<String> {
        let value = self.value.load(Ordering::Relaxed);
        if value == 0 {
            return None;
        }
        let phase = Phase::name(value as u8);
        let age = now_ms.saturating_sub(value >> PHASE_BITS);
        let subject = match &self.subject {
            Subject::World => String::new(),
            Subject::Entity { id, kind, position } => format!(
                ", entity {id} ({kind}), sampled position ({:.2}, {:.2}, {:.2})",
                position.x, position.y, position.z
            ),
            Subject::BlockEntity { kind, position } => format!(
                ", block entity {kind} at ({}, {}, {})",
                position.0.x, position.0.y, position.0.z
            ),
        };
        Some(format!("{phase}, entered {age} ms ago{subject}"))
    }
}

pub struct Group {
    pub world: usize,
    pub epoch: Instant,
    pub records: Vec<Record>,
}

// The snapshots own only copied diagnostics, never players, entities or worlds.
thread_local! {
    static CURRENT: RefCell<Option<(Arc<Group>, usize)>> = const { RefCell::new(None) };
}

pub struct MarkerGroup(pub Option<Arc<Group>>);

impl MarkerGroup {
    /// Marks a subject active until the returned guard is dropped.
    /// Indices follow snapshot order; nested guards must drop in reverse entry order.
    pub fn enter(&self, index: usize, phase: Phase) -> Option<ActiveMarker> {
        let Some(group) = &self.0 else {
            return None;
        };
        group.records[index].enter(phase, elapsed_ms(group.epoch));
        let previous = CURRENT.with_borrow_mut(|current| current.replace((group.clone(), index)));
        Some(ActiveMarker { previous })
    }
}

pub struct ActiveMarker {
    previous: Option<(Arc<Group>, usize)>,
}

impl Drop for ActiveMarker {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|current| {
            if let Some((group, index)) = current.take() {
                group.records[index].finish();
            }
            *current = self.previous.take();
        });
    }
}

pub fn current_world() -> usize {
    CURRENT.with_borrow(|current| current.as_ref().map_or(0, |(group, _)| group.world))
}

pub fn elapsed_ms(epoch: Instant) -> u64 {
    epoch.elapsed().as_millis() as u64
}

pub fn append_groups(report: &mut String, groups: &[Arc<Group>], now_ms: u64) {
    for group in groups {
        for record in &group.records {
            if let Some(line) = record.describe(now_ms) {
                let _ = writeln!(report, "World #{}: {line}", group.world);
            }
        }
    }
}
