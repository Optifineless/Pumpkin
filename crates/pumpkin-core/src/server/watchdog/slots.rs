use std::cell::RefCell;
use std::fmt::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use arc_swap::ArcSwap;
use crossbeam::utils::CachePadded;
use pumpkin_data::{block_properties::BLOCK_ENTITY_TYPES, entity::EntityType};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::version::JavaMinecraftVersion;

use super::markers::{Phase, Record, elapsed_ms};

const PHASE_MASK: u64 = 0xff;
const KIND_SHIFT: u32 = 8;
const TYPE_SHIFT: u32 = 16;
const ID_SHIFT: u32 = 32;
const ENTITY: u64 = 1;
const PLAYER: u64 = 2;
const BLOCK_ENTITY: u64 = 3;

// ServerWatchdog.createWatchdogCrashReport needs worker identities. Unlike Java's
// stack dump, these slots cannot retain or lock any live entity or world.
#[derive(Default)]
pub(super) struct Slot {
    // Phase, subject kind, registry ID and entity ID fit in one atomic word.
    subject: AtomicU64,
    position: AtomicU64,
    world: AtomicUsize,
}

impl Slot {
    fn describe(&self) -> Option<String> {
        let subject = self.subject.load(Ordering::Relaxed);
        if subject == 0 {
            return None;
        }
        let phase = Phase::name(subject as u8);
        let registry_id = (subject >> TYPE_SHIFT) as u16;
        let kind = (subject >> KIND_SHIFT) & PHASE_MASK;
        let identity = match kind {
            ENTITY | PLAYER => {
                let name =
                    EntityType::from_raw(registry_id).map_or("unknown", |kind| kind.resource_name);
                let label = if kind == PLAYER { "player" } else { "entity" };
                format!("{label} {} ({name})", (subject >> ID_SHIFT) as i32)
            }
            BLOCK_ENTITY => {
                let name = BLOCK_ENTITY_TYPES
                    .get(usize::from(registry_id))
                    .copied()
                    .unwrap_or("unknown");
                let position = BlockPos::from_long_for_version(
                    self.position.load(Ordering::Relaxed) as i64,
                    &JavaMinecraftVersion::V_1_14,
                );
                format!(
                    "block entity minecraft:{name} at ({}, {}, {})",
                    position.0.x, position.0.y, position.0.z
                )
            }
            _ => return None,
        };
        Some(format!(
            "World #{}: {phase}, {identity}",
            self.world.load(Ordering::Relaxed)
        ))
    }
}

pub(super) struct Slots {
    registered: ArcSwap<Vec<Arc<CachePadded<Slot>>>>,
    epoch: Instant,
}

impl Slots {
    pub(super) fn new(epoch: Instant) -> Self {
        Self {
            registered: ArcSwap::from_pointee(Vec::new()),
            epoch,
        }
    }

    /// Registers this OS thread once; later ticks reuse its padded atomic slot.
    pub(super) fn register(self: &Arc<Self>) {
        CURRENT.with_borrow_mut(|current| {
            self.worker(current);
        });
    }

    fn worker<'a>(self: &Arc<Self>, current: &'a mut Option<Worker>) -> &'a mut Worker {
        if current
            .as_ref()
            .is_some_and(|worker| !Arc::ptr_eq(&worker.registry, self))
        {
            *current = None;
        }
        current.get_or_insert_with(|| {
            let slot = Arc::new(CachePadded::new(Slot::default()));
            self.registered.rcu(|old| {
                let mut slots = (**old).clone();
                slots.push(slot.clone());
                slots
            });
            Worker {
                registry: self.clone(),
                slot,
                phase: None,
            }
        })
    }

    pub(super) fn scope(
        self: &Arc<Self>,
        world: usize,
        phase: Option<Arc<Record>>,
    ) -> WorkerMarker {
        CURRENT
            .with_borrow_mut(|current| {
                let worker = self.worker(current);
                let previous_phase = std::mem::replace(&mut worker.phase, phase);
                let previous_world = worker.slot.world.load(Ordering::Relaxed);
                let previous_subject = worker.slot.subject.load(Ordering::Relaxed);
                worker.slot.subject.store(0, Ordering::Relaxed);
                worker.slot.world.store(world, Ordering::Relaxed);
                Some(WorkerScope {
                    slot: worker.slot.clone(),
                    previous_phase,
                    previous_world,
                    previous_subject,
                    thread_bound: PhantomData,
                })
            })
            .into()
    }

    pub(super) fn append(&self, report: &mut String) {
        // Relaxed observations are deliberately best-effort during a crash. No
        // retry loop: a wedged writer must never prevent the fatal report.
        for (index, slot) in self.registered.load().iter().enumerate() {
            if let Some(line) = slot.describe() {
                let _ = writeln!(report, "Worker #{index}: {line}");
            }
        }
    }
}

struct Worker {
    registry: Arc<Slots>,
    slot: Arc<CachePadded<Slot>>,
    phase: Option<Arc<Record>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Worker>> = const { RefCell::new(None) };
}

pub(super) fn enter_phase(phase: Phase) {
    CURRENT.with_borrow(|current| {
        if let Some(worker) = current {
            let subject = worker.slot.subject.load(Ordering::Relaxed);
            if subject != 0 {
                worker
                    .slot
                    .subject
                    .store((subject & !PHASE_MASK) | phase as u64, Ordering::Relaxed);
            } else if let Some(record) = &worker.phase {
                record.enter(phase, elapsed_ms(worker.registry.epoch));
            }
        }
    });
}

pub fn current_world() -> usize {
    CURRENT.with_borrow(|current| {
        current
            .as_ref()
            .map_or(0, |worker| worker.slot.world.load(Ordering::Relaxed))
    })
}

// Acquired once per Rayon job/batch, never once per subject. Restores a parent
// scope if nested Rayon work executes on the same worker while it waits.
struct WorkerScope {
    slot: Arc<CachePadded<Slot>>,
    previous_phase: Option<Arc<Record>>,
    previous_world: usize,
    previous_subject: u64,
    thread_bound: PhantomData<Rc<()>>,
}

impl Drop for WorkerScope {
    fn drop(&mut self) {
        self.slot
            .world
            .store(self.previous_world, Ordering::Relaxed);
        self.slot
            .subject
            .store(self.previous_subject, Ordering::Relaxed);
        CURRENT.with_borrow_mut(|current| {
            if let Some(worker) = current {
                if let Some(record) = &worker.phase {
                    record.finish();
                }
                worker.phase = self.previous_phase.take();
            }
        });
    }
}

#[derive(Default)]
pub struct WorkerMarker(Option<WorkerScope>);

impl From<Option<WorkerScope>> for WorkerMarker {
    fn from(scope: Option<WorkerScope>) -> Self {
        Self(scope)
    }
}

impl WorkerMarker {
    /// Marks one entity until drop, using only its copied numeric IDs.
    /// Acquire a worker scope for nested work; subject guards do not nest.
    #[inline]
    pub fn entity(&self, id: i32, registry_id: u16, phase: Phase) -> Option<ActiveSubject<'_>> {
        let worker = self.0.as_ref()?;
        let kind = if matches!(phase, Phase::PlayerTicks) {
            PLAYER
        } else {
            ENTITY
        };
        let subject = (u64::from(id as u32) << ID_SHIFT)
            | (u64::from(registry_id) << TYPE_SHIFT)
            | (kind << KIND_SHIFT)
            | phase as u64;
        worker.slot.subject.store(subject, Ordering::Relaxed);
        Some(ActiveSubject {
            slot: &worker.slot,
            thread_bound: PhantomData,
        })
    }

    /// Marks a block entity using the existing block-state registry ID and position.
    #[inline]
    pub fn block_entity(&self, position: BlockPos, registry_id: u16) -> Option<ActiveSubject<'_>> {
        let worker = self.0.as_ref()?;
        worker
            .slot
            .position
            .store(position.as_long() as u64, Ordering::Relaxed);
        worker.slot.subject.store(
            (u64::from(registry_id) << TYPE_SHIFT)
                | (BLOCK_ENTITY << KIND_SHIFT)
                | Phase::BlockEntityTicks as u64,
            Ordering::Relaxed,
        );
        Some(ActiveSubject {
            slot: &worker.slot,
            thread_bound: PhantomData,
        })
    }
}

pub struct ActiveSubject<'a> {
    slot: &'a Slot,
    thread_bound: PhantomData<&'a WorkerMarker>,
}

impl Drop for ActiveSubject<'_> {
    #[inline]
    fn drop(&mut self) {
        self.slot.subject.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
