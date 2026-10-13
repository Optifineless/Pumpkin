use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use std::{cmp::Reverse, collections::BTreeMap};

use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use crate::tick::{MAX_TICK_DELAY, OrderedTick, SavedTick, ScheduledTick, TickPriority};

use pumpkin_util::sync::{LockClass, Mutex};

enum SchedulerState {}
impl LockClass for SchedulerState {
    const NAME: &'static str = "scheduled tick container";
    // Final unload holds ticket state 30 before freezing; all other callers release tickets first.
    const LEVEL: u8 = 31;
}
enum SaveObservation {}
impl LockClass for SaveObservation {
    const NAME: &'static str = "scheduled tick save observation";
    const LEVEL: u8 = 40;
}

pub(super) type TickKey = (TickPriority, i64, i64);

pub struct ChunkTickScheduler<T> {
    inner: Mutex<ChunkTickSchedulerState<T>, SchedulerState>,
    next_trigger: AtomicI64,
    last_tick: AtomicI64,
    game_time: OnceLock<Arc<AtomicI64>>,
    mutation: AtomicU64,
    // Clock used by the encoded tick image, and last observed queue mutation.
    observed_save_state: Mutex<(i64, u64), SaveObservation>,
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

struct ChunkTickSchedulerState<T> {
    inner: Option<Box<ChunkTickSchedulerInner<T>>>,
    frozen_time: Option<i64>,
}

struct ChunkTickSchedulerInner<T> {
    pending_ticks: Vec<SavedTick<T>>,
    tick_queue: [Vec<PendingTick<T>>; MAX_TICK_DELAY],
    queued_ticks: FxHashSet<(BlockPos, T)>,
    long_ticks: BTreeMap<i64, Vec<PendingTick<T>>>,
    due_ticks: Vec<Reverse<PendingTick<T>>>,
    next_queued_trigger: i64,
}

impl<T> Default for ChunkTickSchedulerInner<T> {
    fn default() -> Self {
        Self {
            pending_ticks: Vec::new(),
            tick_queue: std::array::from_fn(|_| Vec::new()),
            queued_ticks: FxHashSet::default(),
            long_ticks: BTreeMap::new(),
            due_ticks: Vec::new(),
            next_queued_trigger: i64::MAX,
        }
    }
}

fn queue_index(trigger_tick: i64, last_tick: i64) -> usize {
    trigger_tick
        .max(last_tick + 1)
        .rem_euclid(MAX_TICK_DELAY as i64) as usize
}

impl<'a, T> ChunkTickSchedulerInner<&'a T> {
    fn refresh_next_queued_trigger(&mut self) {
        if self.queued_ticks.len() == self.pending_ticks.len() + self.due_ticks.len() {
            self.next_queued_trigger = i64::MAX;
            return;
        }
        self.next_queued_trigger = self
            .tick_queue
            .iter()
            .flatten()
            .map(|tick| tick.trigger_tick)
            .chain(
                self.long_ticks
                    .first_key_value()
                    .map(|(trigger, _)| *trigger),
            )
            .min()
            .unwrap_or(i64::MAX);
    }

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

    /// Retains relative save delays before the chunk leaves the loaded owner.
    pub(crate) fn freeze_for_unload(&self, current_tick: i64) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // ChunkMap.scheduleUnload freezes its saved image before ServerLevel.unload.
        // Stale collectors must never reactivate this object, even without a wheel.
        if guard.frozen_time.is_some() {
            return;
        }
        guard.frozen_time = Some(current_tick);
        self.next_trigger.store(i64::MAX, Ordering::Release);
        let Some(inner) = guard.inner.as_mut() else {
            return;
        };
        if inner.queued_ticks.len() == inner.pending_ticks.len() {
            return;
        }
        // ChunkMap.scheduleUnload -> SerializableChunkData.copyOf freezes relative tick data.
        // A fresh loaded copy restarts these delays through LevelChunkTicks.unpack.
        inner.pending_ticks = inner.saved_ticks(current_tick);
        for queue in &mut inner.tick_queue {
            queue.clear();
        }
        inner.long_ticks.clear();
        inner.due_ticks.clear();
        inner.next_queued_trigger = i64::MAX;
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
            inner.next_queued_trigger = inner.next_queued_trigger.min(trigger_tick);
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

    /// Lock-free eligibility hint, including saved ticks awaiting first simulation.
    pub(crate) fn has_due_ticks(&self, current_tick: i64) -> bool {
        self.next_trigger.load(Ordering::Acquire) <= current_tick
    }

    /// Reports an empty hint; marker removal still rechecks the authoritative set.
    pub(crate) fn is_empty_hint(&self) -> bool {
        self.next_trigger.load(Ordering::Acquire) == i64::MAX
    }

    fn update_next_trigger(&self, inner: &ChunkTickSchedulerInner<&'a T>) {
        // LevelTicks.nextTickForContainer / updateContainerScheduling. Pending delays
        // must unpack on first simulation before their absolute deadline is known.
        let trigger = if inner.pending_ticks.is_empty() {
            inner
                .due_ticks
                .last()
                .map_or(inner.next_queued_trigger, |tick| {
                    tick.0.trigger_tick.min(inner.next_queued_trigger)
                })
        } else {
            i64::MIN
        };
        self.next_trigger.store(trigger, Ordering::Release);
    }

    pub(crate) fn prepare_tick(&self, current_tick: i64) -> Option<(TickKey, usize)> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.frozen_time.is_some() {
            return None;
        }
        let Some(inner) = guard.inner.as_mut() else {
            self.last_tick.store(current_tick, Ordering::Relaxed);
            return None;
        };
        self.unpack(inner, current_tick);
        let previous_tick = self.last_tick.swap(current_tick, Ordering::Relaxed);
        // LevelTicks.sortContainersToTick makes overdue work eligible after tickCheck.
        let elapsed = (current_tick - previous_tick).clamp(0, MAX_TICK_DELAY as i64);
        if inner.next_queued_trigger <= current_tick {
            let mut extracted = std::mem::take(&mut inner.due_ticks);
            let wheel_count: usize = (1..=elapsed)
                .map(|delta| {
                    let index = (previous_tick + delta).rem_euclid(MAX_TICK_DELAY as i64) as usize;
                    inner.tick_queue[index].len()
                })
                .sum();
            let long_count: usize = inner
                .long_ticks
                .range(..=current_tick)
                .map(|(_, queue)| queue.len())
                .sum();
            extracted.reserve(wheel_count + long_count);
            for delta in 1..=elapsed {
                let index = (previous_tick + delta).rem_euclid(MAX_TICK_DELAY as i64) as usize;
                extracted.extend(
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
                    extracted.extend(queue.into_iter().map(Reverse));
                }
            }
            // LevelChunkTicks.poll consumes DRAIN_ORDER. Sort reversed keys once so
            // polling a prepared run moves its minimum from the vector's end.
            extracted.sort_unstable();
            inner.due_ticks = extracted;
            inner.refresh_next_queued_trigger();
        }
        self.update_next_trigger(inner);
        inner
            .due_ticks
            .last()
            .map(|tick| (Self::key(&tick.0), inner.due_ticks.len()))
    }

    const fn key(pending: &PendingTick<&'a T>) -> TickKey {
        (
            pending.tick.priority,
            pending.tick.sub_tick_order,
            pending.trigger_tick,
        )
    }

    /// Moves a due run under one lock; schedules after this container was prepared wait for its next prepare.
    pub(crate) fn poll_due_batch(
        &self,
        limit: usize,
        next_best: Option<(TickPriority, i64)>,
        output: &mut Vec<OrderedTick<&'a T>>,
    ) -> Option<TickKey> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.frozen_time.is_some() {
            return None;
        }
        let inner = guard.inner.as_mut()?;
        let start = output.len();
        // LevelTicks.drainFromCurrentContainer compares INTRA_TICK_DRAIN_ORDER,
        // not trigger times across chunks. Each chunk still uses DRAIN_ORDER.
        // clearArea may replace the cached head after prepare, so recheck the first entry too.
        while output.len() - start < limit
            && inner.due_ticks.last().is_some_and(|head| {
                next_best
                    .is_none_or(|next| (head.0.tick.priority, head.0.tick.sub_tick_order) <= next)
            })
        {
            if let Some(Reverse(pending)) = inner.due_ticks.pop() {
                output.push(pending.tick);
            }
        }
        for tick in &output[start..] {
            inner.queued_ticks.remove(&(tick.position, tick.value));
        }
        if output.len() != start {
            self.mutation.fetch_add(1, Ordering::Relaxed);
        }
        self.update_next_trigger(inner);
        let next = inner.due_ticks.last().map(|tick| Self::key(&tick.0));
        if inner.queued_ticks.is_empty() {
            guard.inner = None;
        }
        next
    }

    pub fn step_tick(&self, current_tick: i64) -> Vec<OrderedTick<&'a T>> {
        let count = self
            .prepare_tick(current_tick)
            .map_or(0, |(_, count)| count);
        let mut due = Vec::with_capacity(count);
        self.poll_due_batch(count, None, &mut due);
        due
    }

    pub(crate) fn mark_dirty_if_changed(&self, dirty: &crate::chunk::io::DirtyFlag) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.observe_dirty(inner.inner.as_deref(), Some(self.current_tick()), dirty);
    }

    /// Observes queue mutations without requesting another elapsed-time snapshot.
    pub(crate) fn mark_dirty_if_mutated(&self, dirty: &crate::chunk::io::DirtyFlag) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.observe_dirty(inner.inner.as_deref(), None, dirty);
    }

    fn observe_dirty(
        &self,
        inner: Option<&ChunkTickSchedulerInner<&'a T>>,
        clock: Option<i64>,
        dirty: &crate::chunk::io::DirtyFlag,
    ) {
        let mut observed = self
            .observed_save_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mutation = self.mutation.load(Ordering::Relaxed);
        let has_runtime_ticks =
            inner.is_some_and(|inner| inner.queued_ticks.len() > inner.pending_ticks.len());
        if observed.1 != mutation {
            dirty.store(true, Ordering::Relaxed);
        }
        // ScheduledTick.toSavedTick changes delays as time passes. Request the next
        // image once; clock progress must not invalidate an image already being written.
        // ScheduledTick.toSavedTick casts the relative delay to a signed 32-bit t.
        if has_runtime_ticks && clock.is_some_and(|clock| observed.0 as i32 != clock as i32) {
            dirty.mark_elapsed_dirty();
        }
        observed.1 = mutation;
    }

    pub fn schedule_tick(&self, tick: &ScheduledTick<&'a T>, sub_tick_order: i64) {
        if !self.schedule_saved_tick(&SavedTick::from(tick.clone()), sub_tick_order) {
            tracing::debug!(position = ?tick.position, "Scheduled tick rejected as duplicate");
        }
    }

    fn schedule_saved_tick(&self, tick: &SavedTick<&'a T>, sub_tick_order: i64) -> bool {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current_tick = self.current_tick();
        let last_tick = self.last_tick.load(Ordering::Relaxed);
        let frozen_time = inner_guard.frozen_time;
        let inner = inner_guard.inner.get_or_insert_with(Box::default);

        if inner.queued_ticks.insert((tick.position, tick.value)) {
            self.mutation.fetch_add(1, Ordering::Relaxed);
            // LevelAccessor.createTick stores game time + delay, even outside simulation range.
            let trigger_tick = current_tick + i64::from(tick.delay);
            if let Some(frozen_time) = frozen_time {
                // LevelChunkTicks.pack / ScheduledTick.toSavedTick: retain the unload clock.
                // A producer holding the public chunk guard finishes before save submission.
                inner.pending_ticks.push(SavedTick {
                    delay: (trigger_tick - frozen_time) as i32,
                    ..tick.clone()
                });
                return true;
            }
            inner.next_queued_trigger = inner.next_queued_trigger.min(trigger_tick);
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
            self.update_next_trigger(inner);
            return true;
        }
        false
    }

    pub fn is_scheduled(&self, pos: BlockPos, value: &T) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .inner
            .as_ref()
            .is_some_and(|inner| inner.queued_ticks.contains(&(pos, value)))
    }

    pub fn clear_area(&self, min: &BlockPos, max: &BlockPos) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retired = inner_guard.frozen_time.is_some();
        let Some(inner) = inner_guard.inner.as_mut() else {
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
        inner.refresh_next_queued_trigger();
        if !retired {
            self.update_next_trigger(inner);
        }
        if inner.queued_ticks.is_empty() {
            inner_guard.inner = None;
        }
    }

    pub fn has_ticks(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .inner
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
        let Some(inner) = inner_guard.inner.as_ref() else {
            return Vec::new();
        };

        inner.saved_ticks(current_tick)
    }

    /// Captures relative delays and remembers the clock used by this encoded image.
    pub(crate) fn to_vec_for_save(
        &self,
        dirty: &crate::chunk::io::DirtyFlag,
    ) -> Vec<SavedTick<&'a T>> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current_tick = self.current_tick();
        self.observe_dirty(inner.inner.as_deref(), Some(current_tick), dirty);
        // SerializableChunkData.copyOf -> LevelChunkTicks.pack snapshots game time
        // with the relative delays. Inspection via to_vec must not advance this clock.
        let ticks = inner
            .inner
            .as_ref()
            .map_or_else(Vec::new, |inner| inner.saved_ticks(current_tick));
        self.observed_save_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0 = current_tick;
        ticks
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
            scheduler
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .inner = Some(Box::new(ChunkTickSchedulerInner {
                pending_ticks,
                queued_ticks,
                tick_queue: std::array::from_fn(|_| Vec::new()),
                long_ticks: BTreeMap::new(),
                due_ticks: Vec::new(),
                next_queued_trigger: i64::MAX,
            }));
            scheduler.next_trigger.store(i64::MIN, Ordering::Release);
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
            inner: Mutex::new(ChunkTickSchedulerState {
                inner: None,
                frozen_time: None,
            }),
            next_trigger: AtomicI64::new(i64::MAX),
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

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "lock order violation")]
    fn scheduler_entry_checks_guards_from_other_fork_owners() {
        enum ExternalLeaf {}
        impl LockClass for ExternalLeaf {
            const NAME: &'static str = "external scheduler test leaf";
            const LEVEL: u8 = 40;
        }
        let leaf = Mutex::<_, ExternalLeaf>::new(());
        let _guard = leaf.lock().unwrap();
        let scheduler = ChunkTickScheduler::<&u8>::default();
        scheduler.step_tick(1);
    }

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
    fn pending_snapshot_clock_observes_encoded_delay_width_and_coalesces_requests() {
        let clock = Arc::new(AtomicI64::new(0));
        let scheduler = ChunkTickScheduler::default();
        scheduler.bind_clock(clock.clone());
        scheduler.schedule_tick(&tick(5000), 0);
        let dirty = crate::chunk::io::DirtyFlag::new(true);
        scheduler.mark_dirty_if_changed(&dirty);
        let version = dirty.version();
        assert_eq!(scheduler.to_vec_for_save(&dirty)[0].delay, 5000);
        assert!(dirty.clear_published(version));
        clock.store(1i64 << 32, Ordering::Relaxed);
        scheduler.mark_dirty_if_changed(&dirty);
        assert_eq!(scheduler.to_vec()[0].delay, 5000);
        assert!(!dirty.load(Ordering::Relaxed));
        clock.store((1i64 << 32) + 1, Ordering::Relaxed);
        scheduler.mark_dirty_if_changed(&dirty);
        let elapsed_version = dirty.version();
        assert!(dirty.load(Ordering::Relaxed));
        assert_ne!(elapsed_version, version);
        clock.store((1i64 << 32) + 2, Ordering::Relaxed);
        scheduler.mark_dirty_if_changed(&dirty);
        assert_eq!(dirty.version(), elapsed_version);
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

#[cfg(test)]
#[path = "scheduler_retirement_tests.rs"]
mod retirement_tests;
