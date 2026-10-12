use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};

use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use crate::tick::{MAX_TICK_DELAY, OrderedTick, SavedTick, ScheduledTick};

pub struct ChunkTickScheduler<T> {
    inner: Mutex<Option<Box<ChunkTickSchedulerInner<T>>>>,
    last_tick: AtomicI64,
    game_time: OnceLock<Arc<AtomicI64>>,
    mutation: AtomicU64,
    observed_save_state: Mutex<(i64, u64)>,
}

struct PendingTick<T> {
    trigger_tick: i64,
    tick: OrderedTick<T>,
}

impl<T> PartialEq for PendingTick<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl<T> Eq for PendingTick<T> {}
impl<T> PartialOrd for PendingTick<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for PendingTick<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // ScheduledTick.DRAIN_ORDER: trigger, priority, then sub-tick order.
        self.trigger_tick
            .cmp(&other.trigger_tick)
            .then_with(|| self.tick.cmp(&other.tick))
    }
}

struct ChunkTickSchedulerInner<T> {
    pending_ticks: Vec<SavedTick<T>>,
    tick_queue: [Vec<PendingTick<T>>; MAX_TICK_DELAY],
    queued_ticks: FxHashSet<(BlockPos, T)>,
    long_ticks: BTreeMap<i64, Vec<PendingTick<T>>>,
    due_ticks: BinaryHeap<Reverse<PendingTick<T>>>,
}

fn queue_index(trigger_tick: i64, last_tick: i64) -> usize {
    trigger_tick
        .max(last_tick + 1)
        .rem_euclid(MAX_TICK_DELAY as i64) as usize
}

impl<'a, T> ChunkTickSchedulerInner<&'a T> {
    fn saved_ticks(&self, current_tick: i64) -> Vec<SavedTick<&'a T>> {
        // LevelChunkTicks.pack / ScheduledTick.toSavedTick keep negative overdue delays.
        let mut pending: Vec<_> = self
            .tick_queue
            .iter()
            .chain(self.long_ticks.values())
            .flatten()
            .chain(self.due_ticks.iter().map(|pending| &pending.0))
            .collect();
        pending.sort_unstable_by_key(|pending| pending.tick.sub_tick_order);
        self.pending_ticks
            .iter()
            .cloned()
            .chain(pending.into_iter().map(|pending| SavedTick {
                delay: (pending.trigger_tick - current_tick) as i32,
                priority: pending.tick.priority,
                position: pending.tick.position,
                value: pending.tick.value,
            }))
            .collect()
    }
}

impl<'a, T: std::hash::Hash + Eq> ChunkTickScheduler<&'a T> {
    /// Attaches the clock without starting saved delays before first simulation.
    pub(crate) fn bind_clock(&self, game_time: Arc<AtomicI64>) {
        let _ = self.game_time.set(game_time);
    }

    /// Retains relative save delays after the chunk leaves the loaded owner.
    pub(crate) fn freeze_for_unload(&self, current_tick: i64) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else {
            return;
        };
        if inner.queued_ticks.len() == inner.pending_ticks.len() {
            return;
        }
        // ChunkMap.scheduleUnload -> SerializableChunkData.copyOf freezes relative tick data.
        // A retained original must reload through LevelChunkTicks.unpack at first simulation.
        inner.pending_ticks = inner.saved_ticks(current_tick);
        for queue in &mut inner.tick_queue {
            queue.clear();
        }
        inner.long_ticks.clear();
        inner.due_ticks.clear();
        self.mutation.fetch_add(1, Ordering::Relaxed);
    }

    fn unpack(&self, inner: &mut ChunkTickSchedulerInner<&'a T>, current_tick: i64) {
        if inner.pending_ticks.is_empty() {
            return;
        }
        // ServerLevel.startTickingChunk -> LevelChunkTicks.unpack: delays start at first simulation.
        let sub_tick_base = -(inner.pending_ticks.len() as i64);
        let last_tick = self.last_tick.load(Ordering::Relaxed).min(current_tick - 1);
        self.last_tick.store(last_tick, Ordering::Relaxed);
        for (index, tick) in inner.pending_ticks.drain(..).enumerate() {
            let trigger_tick = current_tick + i64::from(tick.delay);
            let queue = if tick.delay > MAX_TICK_DELAY as i32 {
                inner.long_ticks.entry(trigger_tick).or_default()
            } else {
                &mut inner.tick_queue[queue_index(trigger_tick, last_tick)]
            };
            queue.push(PendingTick {
                trigger_tick,
                tick: OrderedTick {
                    priority: tick.priority,
                    sub_tick_order: sub_tick_base + index as i64,
                    position: tick.position,
                    value: tick.value,
                },
            });
        }
        self.mutation.fetch_add(1, Ordering::Relaxed);
    }

    fn current_tick(&self) -> i64 {
        self.game_time.get().map_or_else(
            || self.last_tick.load(Ordering::Relaxed),
            |time| time.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn prepare_tick(&self, current_tick: i64) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = inner_guard.as_mut() else {
            self.last_tick.store(current_tick, Ordering::Relaxed);
            return;
        };
        self.unpack(inner, current_tick);
        let previous_tick = self.last_tick.swap(current_tick, Ordering::Relaxed);

        // LevelTicks.sortContainersToTick makes overdue work eligible after tickCheck.
        // Scan one wheel slot normally, at most one whole wheel after a simulation gap.
        let elapsed = (current_tick - previous_tick).clamp(0, MAX_TICK_DELAY as i64);
        for delta in 1..=elapsed {
            let index = (previous_tick + delta).rem_euclid(MAX_TICK_DELAY as i64) as usize;
            inner.due_ticks.extend(
                inner.tick_queue[index]
                    .extract_if(.., |tick| tick.trigger_tick <= current_tick)
                    .map(Reverse),
            );
        }
        while inner
            .long_ticks
            .first_key_value()
            .is_some_and(|(trigger, _)| *trigger <= current_tick)
        {
            if let Some((_, queue)) = inner.long_ticks.pop_first() {
                inner.due_ticks.extend(queue.into_iter().map(Reverse));
            }
        }
    }

    pub(crate) fn peek_due_tick(&self) -> Option<OrderedTick<&'a T>> {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()?
            .due_ticks
            .peek()
            .map(|pending| pending.0.tick.clone())
    }

    pub(crate) fn poll_due_tick(&self) -> Option<OrderedTick<&'a T>> {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let inner = inner_guard.as_mut()?;
        let Reverse(pending) = inner.due_ticks.pop()?;
        inner
            .queued_ticks
            .remove(&(pending.tick.position, pending.tick.value));
        self.mutation.fetch_add(1, Ordering::Relaxed);
        if inner.queued_ticks.is_empty() {
            *inner_guard = None;
        }
        Some(pending.tick)
    }

    pub fn step_tick(&self, current_tick: i64) -> Vec<OrderedTick<&'a T>> {
        self.prepare_tick(current_tick);
        std::iter::from_fn(|| self.poll_due_tick()).collect()
    }

    pub(crate) fn mark_dirty_if_changed(&self, dirty: &crate::chunk::io::DirtyFlag) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut observed = self
            .observed_save_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = (self.current_tick(), self.mutation.load(Ordering::Relaxed));
        // ScheduledTick.toSavedTick uses trigger - game time. Queue mutations and
        // elapsed-only changes must reach Pumpkin's versioned publication barrier.
        let has_runtime_ticks = inner
            .as_ref()
            .is_some_and(|inner| inner.queued_ticks.len() > inner.pending_ticks.len());
        if observed.1 != state.1 || (has_runtime_ticks && observed.0 != state.0) {
            dirty.store(true, Ordering::Relaxed);
        }
        *observed = state;
    }

    pub fn schedule_tick(&self, tick: &ScheduledTick<&'a T>, sub_tick_order: i64) {
        self.schedule_saved_tick(&SavedTick::from(tick.clone()), sub_tick_order);
    }

    fn schedule_saved_tick(&self, tick: &SavedTick<&'a T>, sub_tick_order: i64) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current_tick = self.current_tick();
        let last_tick = self.last_tick.load(Ordering::Relaxed);
        let inner = inner_guard.get_or_insert_with(|| {
            Box::new(ChunkTickSchedulerInner {
                pending_ticks: Vec::new(),
                tick_queue: std::array::from_fn(|_| Vec::new()),
                queued_ticks: FxHashSet::default(),
                long_ticks: BTreeMap::new(),
                due_ticks: BinaryHeap::new(),
            })
        });

        if inner.queued_ticks.insert((tick.position, tick.value)) {
            self.mutation.fetch_add(1, Ordering::Relaxed);
            // LevelAccessor.createTick stores game time + delay, even outside simulation range.
            let trigger_tick = current_tick + i64::from(tick.delay);
            let queue = if tick.delay > MAX_TICK_DELAY as i32 {
                inner.long_ticks.entry(trigger_tick).or_default()
            } else {
                &mut inner.tick_queue[queue_index(trigger_tick, last_tick)]
            };
            queue.push(PendingTick {
                trigger_tick,
                tick: OrderedTick {
                    priority: tick.priority,
                    sub_tick_order,
                    position: tick.position,
                    value: tick.value,
                },
            });
        }
    }

    pub fn is_scheduled(&self, pos: BlockPos, value: &T) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|inner| inner.queued_ticks.contains(&(pos, value)))
    }

    pub fn clear_area(&self, min: &BlockPos, max: &BlockPos) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = inner_guard.as_mut() else {
            return;
        };

        let contains = |position: &BlockPos| {
            position.0.x >= min.0.x
                && position.0.x < max.0.x
                && position.0.y >= min.0.y
                && position.0.y < max.0.y
                && position.0.z >= min.0.z
                && position.0.z < max.0.z
        };

        for queue in &mut inner.tick_queue {
            queue.retain(|pending| !contains(&pending.tick.position));
        }
        inner.long_ticks.retain(|_, queue| {
            queue.retain(|pending| !contains(&pending.tick.position));
            !queue.is_empty()
        });
        let previous_count = inner.queued_ticks.len();
        inner.pending_ticks.retain(|tick| !contains(&tick.position));
        inner
            .due_ticks
            .retain(|pending| !contains(&pending.0.tick.position));
        inner
            .queued_ticks
            .retain(|(position, _)| !contains(position));
        if inner.queued_ticks.len() != previous_count {
            self.mutation.fetch_add(1, Ordering::Relaxed);
        }
        if inner.queued_ticks.is_empty() {
            *inner_guard = None;
        }
    }

    pub fn has_ticks(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|inner| !inner.queued_ticks.is_empty())
    }

    #[must_use]
    pub fn to_vec(&self) -> Vec<SavedTick<&'a T>> {
        let inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current_tick = self.current_tick();
        let Some(inner) = inner_guard.as_ref() else {
            return Vec::new();
        };

        inner.saved_ticks(current_tick)
    }
}

impl<'a, T: std::hash::Hash + Eq + 'static> FromIterator<SavedTick<&'a T>>
    for ChunkTickScheduler<&'a T>
{
    fn from_iter<I: IntoIterator<Item = SavedTick<&'a T>>>(iter: I) -> Self {
        let scheduler = Self::default();
        let ticks: Vec<_> = iter.into_iter().collect();
        if !ticks.is_empty() {
            // LevelChunkTicks(List<SavedTick>) retains relative delays until unpack.
            let mut queued_ticks = FxHashSet::default();
            let pending_ticks = ticks
                .into_iter()
                .filter(|tick| queued_ticks.insert((tick.position, tick.value)))
                .collect();
            *scheduler
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(Box::new(ChunkTickSchedulerInner {
                    pending_ticks,
                    queued_ticks,
                    tick_queue: std::array::from_fn(|_| Vec::new()),
                    long_ticks: BTreeMap::new(),
                    due_ticks: BinaryHeap::new(),
                }));
        }
        scheduler
    }
}

impl<'a, T: std::hash::Hash + Eq + 'static> FromIterator<ScheduledTick<&'a T>>
    for ChunkTickScheduler<&'a T>
{
    fn from_iter<I: IntoIterator<Item = ScheduledTick<&'a T>>>(iter: I) -> Self {
        iter.into_iter().map(SavedTick::from).collect()
    }
}

impl<T> Default for ChunkTickScheduler<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            last_tick: AtomicI64::new(0),
            game_time: OnceLock::new(),
            mutation: AtomicU64::new(0),
            observed_save_state: Mutex::new((0, 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tick::TickPriority;

    static BLOCK: u8 = 0;

    fn tick(delay: u32) -> ScheduledTick<&'static u8> {
        ScheduledTick {
            delay,
            priority: TickPriority::Normal,
            position: BlockPos::new(0, 0, 0),
            value: &BLOCK,
        }
    }

    #[test]
    fn delay_counts_game_ticks_like_vanilla() {
        // Vanilla: scheduled at game time T with delay 2, it runs at T + 2.
        let scheduler = ChunkTickScheduler::default();
        scheduler.schedule_tick(&tick(2), 0);
        assert!(scheduler.step_tick(1).is_empty());
        assert_eq!(scheduler.step_tick(2).len(), 1);
    }

    #[test]
    fn saved_delay_round_trips() {
        let scheduler = ChunkTickScheduler::default();
        scheduler.schedule_tick(&tick(5), 0);
        scheduler.step_tick(1);
        assert_eq!(scheduler.to_vec()[0].delay, 4);
    }

    #[test]
    fn mixed_queue_reload_preserves_order_before_fresh_ticks() {
        let scheduler = ChunkTickScheduler::default();
        let a = BlockPos::new(0, 0, 0);
        let b = BlockPos::new(1, 0, 0);
        let fresh = BlockPos::new(2, 0, 0);
        scheduler.schedule_tick(&tick(5000), 100);
        for time in 1..=4800 {
            assert!(scheduler.step_tick(time).is_empty());
        }
        scheduler.schedule_tick(
            &ScheduledTick {
                position: b,
                ..tick(200)
            },
            101,
        );
        let saved = scheduler.to_vec();
        assert_eq!(
            saved.iter().map(|tick| tick.position).collect::<Vec<_>>(),
            [a, b]
        );
        let scheduler: ChunkTickScheduler<_> = saved.into_iter().collect();
        assert!(scheduler.step_tick(0).is_empty());
        scheduler.schedule_tick(
            &ScheduledTick {
                position: fresh,
                ..tick(200)
            },
            0,
        );
        for time in 1..200 {
            assert!(scheduler.step_tick(time).is_empty());
        }
        let mut due = scheduler.step_tick(200);
        due.sort_unstable();
        assert_eq!(
            due.iter().map(|tick| tick.position).collect::<Vec<_>>(),
            [a, b, fresh]
        );
        assert!(due[0].sub_tick_order < due[1].sub_tick_order);
        assert!(due[1].sub_tick_order < 0);
    }
    #[test]
    fn dried_ghast_delay_survives_save_and_tick_wheel_wraps() {
        let scheduler = ChunkTickScheduler::default();
        scheduler.schedule_tick(&tick(5000), 0);
        for time in 1..=300 {
            assert!(scheduler.step_tick(time).is_empty());
        }
        let saved = scheduler.to_vec();
        assert_eq!(saved[0].delay, 4700);
        let scheduler: ChunkTickScheduler<_> = saved.into_iter().collect();
        assert!(scheduler.step_tick(0).is_empty());
        for time in 1..4700 {
            assert!(scheduler.step_tick(time).is_empty());
        }
        assert_eq!(scheduler.step_tick(4700).len(), 1);
        assert!(!scheduler.has_ticks());
        scheduler.schedule_tick(&tick(1), 1);
        assert_eq!(scheduler.step_tick(4701).len(), 1);
    }

    #[test]
    fn clearing_area_removes_long_delays() {
        let scheduler = ChunkTickScheduler::default();
        scheduler.schedule_tick(&tick(5000), 0);
        let outside = BlockPos::new(2, 0, 0);
        scheduler.schedule_tick(
            &ScheduledTick {
                position: outside,
                ..tick(5000)
            },
            1,
        );
        scheduler.clear_area(&BlockPos::new(0, 0, 0), &BlockPos::new(1, 1, 1));
        let remaining = scheduler.to_vec();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].position, outside);
    }
}
