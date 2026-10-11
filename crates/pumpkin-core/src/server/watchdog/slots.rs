use std::cell::RefCell;
use std::fmt::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
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
    fn append(&self, report: &mut impl Write) -> std::fmt::Result {
        let subject = self.subject.load(Ordering::Relaxed);
        if subject == 0 {
            return Ok(());
        }
        let phase = Phase::name(subject as u8);
        let registry_id = (subject >> TYPE_SHIFT) as u16;
        let kind = (subject >> KIND_SHIFT) & PHASE_MASK;
        write!(report, ": {phase}, ")?;
        match kind {
            ENTITY | PLAYER => {
                let name =
                    EntityType::from_raw(registry_id).map_or("unknown", |kind| kind.resource_name);
                let label = if kind == PLAYER { "player" } else { "entity" };
                writeln!(report, "{label} {} ({name})", (subject >> ID_SHIFT) as i32)
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
                writeln!(
                    report,
                    "block entity minecraft:{name} at ({}, {}, {})",
                    position.0.x, position.0.y, position.0.z
                )
            }
            _ => Ok(()),
        }
    }
}

pub(super) struct Slots {
    registered: ArcSwap<Vec<Arc<CachePadded<Slot>>>>,
    epoch: Instant,
}

impl Slots {
    pub(super) fn warm_reader(&self) {
        drop(self.registered.load());
    }

    pub(super) fn new(epoch: Instant) -> Self {
        Self {
            registered: ArcSwap::from_pointee(Vec::new()),
            epoch,
        }
    }

    /// Registers this OS thread once; later ticks reuse its padded atomic slot.
    pub(super) fn register(self: &Arc<Self>) {
        CURRENT.with_borrow_mut(|current| {
            if current
                .as_ref()
                .is_some_and(|worker| !Arc::ptr_eq(&worker.registry, self))
            {
                // Preparing another server must not replace a live scope's TLS.
                self.worker(&mut None);
            } else {
                self.worker(current);
            }
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
            let slot = CACHED.with_borrow_mut(|cached| {
                cached.retain(|entry| entry.registry.strong_count() != 0);
                if let Some(entry) = cached
                    .iter()
                    .find(|entry| entry.registry.ptr_eq(&Arc::downgrade(self)))
                {
                    return entry.slot.clone();
                }
                let slot = Arc::new(CachePadded::new(Slot::default()));
                self.registered.rcu(|old| {
                    let mut slots = (**old).clone();
                    slots.push(slot.clone());
                    slots
                });
                cached.push(CachedSlot {
                    registry: Arc::downgrade(self),
                    slot: slot.clone(),
                });
                slot
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
                let previous_worker = current
                    .as_ref()
                    .is_some_and(|worker| !Arc::ptr_eq(&worker.registry, self))
                    .then(|| current.take())
                    .flatten();
                let worker = self.worker(current);
                let previous_phase = std::mem::replace(&mut worker.phase, phase);
                let previous_phase_value = previous_phase.as_ref().and_then(|parent| {
                    worker
                        .phase
                        .as_ref()
                        .and_then(|child| Arc::ptr_eq(parent, child).then(|| parent.snapshot()))
                });
                let previous_world = worker.slot.world.load(Ordering::Relaxed);
                let previous_subject = worker.slot.subject.load(Ordering::Relaxed);
                let previous_position = worker.slot.position.load(Ordering::Relaxed);
                worker.slot.subject.store(0, Ordering::Relaxed);
                worker.slot.world.store(world, Ordering::Relaxed);
                Some(WorkerScope {
                    slot: worker.slot.clone(),
                    previous_worker,
                    previous_phase,
                    previous_phase_value,
                    previous_world,
                    previous_subject,
                    previous_position,
                    thread_bound: PhantomData,
                })
            })
            .into()
    }

    pub(super) fn append(
        &self,
        report: &mut impl Write,
        world_id: impl Fn(usize) -> Option<uuid::Uuid>,
    ) {
        // Relaxed observations are deliberately best-effort during a crash. No
        // retry loop: a wedged writer must never prevent the fatal report.
        for (index, slot) in self.registered.load().iter().enumerate() {
            if slot.subject.load(Ordering::Relaxed) != 0 {
                let world = slot.world.load(Ordering::Relaxed);
                let _ = write!(report, "Worker #{index}: World #{world}");
                if let Some(id) = world_id(world) {
                    let _ = write!(report, " [{id}]");
                }
                let _ = slot.append(report);
            }
        }
    }
}

struct Worker {
    registry: Arc<Slots>,
    slot: Arc<CachePadded<Slot>>,
    phase: Option<Arc<Record>>,
}

struct CachedSlot {
    // The cache must not keep an unloaded server alive on long-lived Rayon threads.
    registry: Weak<Slots>,
    slot: Arc<CachePadded<Slot>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Worker>> = const { RefCell::new(None) };
    static CACHED: RefCell<Vec<CachedSlot>> = const { RefCell::new(Vec::new()) };
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
    previous_worker: Option<Worker>,
    previous_phase: Option<Arc<Record>>,
    previous_phase_value: Option<u64>,
    previous_world: usize,
    previous_subject: u64,
    previous_position: u64,
    thread_bound: PhantomData<Rc<()>>,
}

impl Drop for WorkerScope {
    fn drop(&mut self) {
        // Restore coordinates before publishing the parent subject.
        self.slot
            .position
            .store(self.previous_position, Ordering::Relaxed);
        self.slot
            .world
            .store(self.previous_world, Ordering::Relaxed);
        self.slot
            .subject
            .store(self.previous_subject, Ordering::Relaxed);
        CURRENT.with_borrow_mut(|current| {
            if let Some(worker) = current {
                if let Some(record) = &worker.phase {
                    if let Some(value) = self.previous_phase_value {
                        record.restore(value);
                    } else {
                        record.finish();
                    }
                }
                worker.phase = self.previous_phase.take();
            }
            if let Some(previous) = self.previous_worker.take() {
                *current = Some(previous);
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
