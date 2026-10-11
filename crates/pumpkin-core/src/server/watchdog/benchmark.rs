use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use arc_swap::ArcSwap;
use crossbeam::utils::CachePadded;
use pumpkin_data::{block_properties::BLOCK_ENTITY_TYPES, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rayon::prelude::*;

use super::markers::{Phase, Record};
use super::slots::Slots;

#[expect(
    dead_code,
    reason = "Reference implementation for the manual benchmark."
)]
#[path = "benchmark_legacy.rs"]
mod legacy;

const SUBJECTS: usize = 2100;
const TICKS: usize = 10_000;
const SAMPLES: usize = 9;
const BATCH: usize = 16;
const ALLOCATION_TICKS: usize = 100;

thread_local! {
    static COUNT_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
}
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

struct Counter;

#[global_allocator]
static ALLOCATOR: Counter = Counter;

fn count_allocation() {
    if COUNT_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: forwards each unchanged layout/pointer to System, with diagnostic counting.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies GlobalAlloc's valid layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies GlobalAlloc's valid layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout are passed through from the allocating caller.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies GlobalAlloc's valid allocation and size.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Baseline,
    Legacy,
    Slots,
    StoreFloor,
}

struct Fixture {
    pool: rayon::ThreadPool,
    slots: Arc<Slots>,
    world: Arc<Record>,
    legacy: ArcSwap<Vec<Arc<legacy::Group>>>,
    floor: Vec<CachePadded<AtomicU64>>,
    epoch: Instant,
    hopper_id: u16,
}

impl Fixture {
    fn new() -> Result<Self, rayon::ThreadPoolBuildError> {
        let epoch = Instant::now();
        let slots = Arc::new(Slots::new(epoch));
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
        pool.broadcast(|_| slots.register());
        slots.register();
        Ok(Self {
            pool,
            slots,
            world: Arc::new(Record::new()),
            legacy: ArcSwap::from_pointee(Vec::new()),
            floor: (0..4)
                .map(|_| CachePadded::new(AtomicU64::new(0)))
                .collect(),
            epoch,
            hopper_id: BLOCK_ENTITY_TYPES
                .iter()
                .position(|name| *name == "hopper")
                .unwrap_or_default() as u16,
        })
    }

    #[expect(
        clippy::needless_collect,
        reason = "Measures the pre-slot production collector."
    )]
    fn legacy_group(&self, count: i32, block: bool) -> legacy::MarkerGroup {
        // observations.rs collected Subject first, then group_in_world made Records.
        let subjects: Vec<_> = (0..count)
            .map(|id| {
                if block {
                    legacy::Subject::BlockEntity {
                        kind: "minecraft:hopper",
                        position: BlockPos::new(id, 64, -id),
                    }
                } else {
                    legacy::Subject::Entity {
                        id,
                        kind: "minecart",
                        position: Vector3::new(f64::from(id), 64.0, 0.0),
                    }
                }
            })
            .collect();
        let group = Arc::new(legacy::Group {
            world: 0,
            epoch: self.epoch,
            records: subjects.into_iter().map(legacy::Record::new).collect(),
        });
        self.legacy.rcu(|old| {
            let mut groups = (**old).clone();
            groups.push(group.clone());
            groups
        });
        legacy::MarkerGroup(Some(group))
    }

    fn run(&self, mode: Mode, ticks: usize, count_allocations: bool) -> f64 {
        let start = Instant::now();
        for _ in 0..ticks {
            if count_allocations {
                COUNT_ALLOCATIONS.set(true);
            }
            let world_marker =
                matches!(mode, Mode::Slots).then(|| self.slots.scope(0, Some(self.world.clone())));
            if matches!(mode, Mode::Legacy) {
                self.legacy.store(Arc::new(Vec::new()));
            }
            for (count, phase) in [
                (1000, Phase::EntityTicks),
                (100, Phase::PlayerTicks),
                (1000, Phase::BlockEntityTicks),
            ] {
                if count_allocations {
                    COUNT_ALLOCATIONS.set(true);
                }
                let old = if matches!(mode, Mode::Legacy) {
                    self.legacy_group(count, matches!(phase, Phase::BlockEntityTicks))
                } else {
                    legacy::MarkerGroup(None)
                };
                if matches!(mode, Mode::Slots) {
                    phase.enter();
                }
                // External pool injection allocates queue blocks independently of
                // the watchdog. Count producer work and all worker work instead.
                if count_allocations {
                    COUNT_ALLOCATIONS.set(false);
                }
                self.pool.install(|| {
                    (0..count as usize)
                        .into_par_iter()
                        .with_min_len(BATCH)
                        .for_each_init(
                            || {
                                (
                                    matches!(mode, Mode::Slots).then(|| self.slots.scope(0, None)),
                                    rayon::current_thread_index(),
                                )
                            },
                            |(marker, worker), index| {
                                self.subject(mode, marker.as_ref(), &old, *worker, index, phase);
                            },
                        );
                });
            }
            if count_allocations {
                COUNT_ALLOCATIONS.set(true);
            }
            drop(world_marker);
            if count_allocations {
                COUNT_ALLOCATIONS.set(false);
            }
        }
        start.elapsed().as_secs_f64() * 1e9 / ticks as f64
    }

    fn subject(
        &self,
        mode: Mode,
        marker: Option<&super::slots::WorkerMarker>,
        old: &legacy::MarkerGroup,
        worker: Option<usize>,
        index: usize,
        phase: Phase,
    ) {
        match mode {
            Mode::Baseline => {
                black_box(index);
            }
            Mode::Slots => {
                if let Some(marker) = marker {
                    let _subject = if matches!(phase, Phase::BlockEntityTicks) {
                        marker.block_entity(
                            BlockPos::new(index as i32, 64, -(index as i32)),
                            self.hopper_id,
                        )
                    } else {
                        marker.entity(index as i32, EntityType::MINECART.id, phase)
                    };
                    if matches!(phase, Phase::PlayerTicks) {
                        Phase::Packets.enter();
                        Phase::PlayerTicks.enter();
                    }
                    black_box(index);
                }
            }
            Mode::Legacy => {
                let phase = if matches!(phase, Phase::PlayerTicks) {
                    legacy::Phase::PlayerTicks
                } else if matches!(phase, Phase::BlockEntityTicks) {
                    legacy::Phase::BlockEntityTicks
                } else {
                    legacy::Phase::EntityTicks
                };
                let _guard = old.enter(index, phase);
                if matches!(phase, legacy::Phase::PlayerTicks) {
                    legacy::Phase::Packets.enter();
                    legacy::Phase::PlayerTicks.enter();
                }
                black_box(index);
            }
            Mode::StoreFloor => {
                if let Some(worker) = worker {
                    self.floor[worker].store(index as u64 + 1, Ordering::Relaxed);
                    black_box(index);
                    self.floor[worker].store(0, Ordering::Relaxed);
                }
            }
        }
    }

    fn allocations(&self, mode: Mode) -> u64 {
        self.pool.broadcast(|_| COUNT_ALLOCATIONS.set(true));
        ALLOCATIONS.store(0, Ordering::Relaxed);
        self.run(mode, ALLOCATION_TICKS, true);
        let measured = ALLOCATIONS.load(Ordering::Relaxed);
        self.pool.broadcast(|_| COUNT_ALLOCATIONS.set(false));
        measured
    }
}

#[test]
#[ignore = "Manual release benchmark: run alone with --release --ignored --nocapture"]
#[expect(clippy::print_stdout, reason = "Manual benchmark results.")]
fn synthetic_2100_subject_tick() -> Result<(), rayon::ThreadPoolBuildError> {
    assert!(
        !black_box(cfg!(debug_assertions)),
        "Use the release profile for performance measurements"
    );
    let fixture = Fixture::new()?;
    let modes = [Mode::Baseline, Mode::Legacy, Mode::Slots, Mode::StoreFloor];
    for mode in modes {
        fixture.run(mode, 1000, false);
    }
    let allocations = modes.map(|mode| fixture.allocations(mode));
    assert_eq!(
        allocations[0], 0,
        "Warm Rayon baseline must allocate nothing"
    );
    assert_eq!(
        allocations[2], 0,
        "Watchdog slots must allocate nothing per tick"
    );
    let mut samples = [[0.0; SAMPLES]; 4];
    for sample in 0..SAMPLES {
        for offset in 0..4 {
            let index = (sample + offset) % 4;
            samples[index][sample] = fixture.run(modes[index], TICKS, false);
        }
        println!(
            "sample {sample}: baseline/legacy/slots/floor ns per tick: {:?}",
            samples.map(|values| values[sample])
        );
    }
    for (index, mode) in modes.iter().enumerate() {
        let mut delta = std::array::from_fn::<_, SAMPLES, _>(|sample| {
            samples[index][sample] - samples[0][sample]
        });
        delta.sort_by(f64::total_cmp);
        let mut times = samples[index];
        times.sort_by(f64::total_cmp);
        println!(
            "{mode:?}: median {:.1} ns/tick; paired delta {:.1} ns/tick, {:.3} ns/subject; {:.3} allocations/tick",
            times[SAMPLES / 2],
            delta[SAMPLES / 2],
            delta[SAMPLES / 2] / SUBJECTS as f64,
            allocations[index] as f64 / ALLOCATION_TICKS as f64
        );
    }
    Ok(())
}
