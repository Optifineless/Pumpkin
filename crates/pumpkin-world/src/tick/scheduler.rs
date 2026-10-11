use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicI64, Ordering},
};

use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use crate::tick::{MAX_TICK_DELAY, OrderedTick, SavedTick, ScheduledTick};

pub struct ChunkTickScheduler<T> {
    inner: Mutex<Option<Box<ChunkTickSchedulerInner<T>>>>,
    last_tick: AtomicI64,
    game_time: OnceLock<Arc<AtomicI64>>,
}

struct PendingTick<T> {
    trigger_tick: i64,
    tick: OrderedTick<T>,
}

struct ChunkTickSchedulerInner<T> {
    tick_queue: [Vec<PendingTick<T>>; MAX_TICK_DELAY],
    queued_ticks: FxHashSet<(BlockPos, T)>,
    long_ticks: BTreeMap<i64, Vec<PendingTick<T>>>,
}

fn queue_index(trigger_tick: i64, last_tick: i64) -> usize {
    trigger_tick
        .max(last_tick + 1)
        .rem_euclid(MAX_TICK_DELAY as i64) as usize
}

impl<'a, T: std::hash::Hash + Eq> ChunkTickScheduler<&'a T> {
    /// Attaches saved delays to the level clock once, when the chunk is published.
    pub(crate) fn unpack(&self, game_time: Arc<AtomicI64>) {
        if self.game_time.get().is_some() {
            return;
        }
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.game_time.get().is_some() {
            return;
        }
        let current_tick = game_time.load(Ordering::Relaxed);
        let shift = current_tick - self.last_tick.load(Ordering::Relaxed);
        let last_tick = current_tick - 1;
        self.last_tick.store(last_tick, Ordering::Relaxed);
        // LevelChunkTicks.unpack / SavedTick.unpack: saved delays start at the load time.
        if let Some(inner) = inner_guard.as_mut() {
            let mut queues =
                std::mem::replace(&mut inner.tick_queue, std::array::from_fn(|_| Vec::new()));
            for pending in queues.iter_mut().flat_map(|queue| queue.drain(..)) {
                let pending = PendingTick {
                    trigger_tick: pending.trigger_tick + shift,
                    ..pending
                };
                let index = queue_index(pending.trigger_tick, last_tick);
                inner.tick_queue[index].push(pending);
            }
            inner.long_ticks = std::mem::take(&mut inner.long_ticks)
                .into_iter()
                .map(|(trigger, mut queue)| {
                    for pending in &mut queue {
                        pending.trigger_tick += shift;
                    }
                    (trigger + shift, queue)
                })
                .collect();
        }
        let _ = self.game_time.set(game_time);
    }

    fn current_tick(&self) -> i64 {
        self.game_time.get().map_or_else(
            || self.last_tick.load(Ordering::Relaxed),
            |time| time.load(Ordering::Relaxed),
        )
    }

    pub fn step_tick(&self, current_tick: i64) -> Vec<OrderedTick<&'a T>> {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_tick = self.last_tick.swap(current_tick, Ordering::Relaxed);
        let Some(inner) = inner_guard.as_mut() else {
            return Vec::new();
        };

        // LevelTicks.sortContainersToTick drains every trigger <= game time, after tickCheck.
        // Scan one wheel slot normally, at most one whole wheel after a simulation gap.
        let elapsed = (current_tick - previous_tick).clamp(0, MAX_TICK_DELAY as i64);
        let mut due = Vec::new();
        for delta in 1..=elapsed {
            let index = (previous_tick + delta).rem_euclid(MAX_TICK_DELAY as i64) as usize;
            due.extend(
                inner.tick_queue[index].extract_if(.., |tick| tick.trigger_tick <= current_tick),
            );
        }
        while inner
            .long_ticks
            .first_key_value()
            .is_some_and(|(trigger, _)| *trigger <= current_tick)
        {
            if let Some((_, mut queue)) = inner.long_ticks.pop_first() {
                due.append(&mut queue);
            }
        }

        // ScheduledTick.DRAIN_ORDER preserves trigger order inside each chunk.
        due.sort_unstable_by(|a, b| {
            a.trigger_tick
                .cmp(&b.trigger_tick)
                .then_with(|| a.tick.cmp(&b.tick))
        });
        for pending in &due {
            inner
                .queued_ticks
                .remove(&(pending.tick.position, pending.tick.value));
        }
        if inner.queued_ticks.is_empty() {
            *inner_guard = None;
        }
        due.into_iter().map(|pending| pending.tick).collect()
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
                tick_queue: std::array::from_fn(|_| Vec::new()),
                queued_ticks: FxHashSet::default(),
                long_ticks: BTreeMap::new(),
            })
        });

        if inner.queued_ticks.insert((tick.position, tick.value)) {
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
        inner
            .queued_ticks
            .retain(|(position, _)| !contains(position));
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

        // LevelChunkTicks.pack / ScheduledTick.toSavedTick keep negative overdue delays.
        let mut pending: Vec<_> = inner
            .tick_queue
            .iter()
            .chain(inner.long_ticks.values())
            .flatten()
            .collect();
        pending.sort_unstable_by_key(|pending| pending.tick.sub_tick_order);
        pending
            .into_iter()
            .map(|pending| SavedTick {
                delay: (pending.trigger_tick - current_tick) as i32,
                priority: pending.tick.priority,
                position: pending.tick.position,
                value: pending.tick.value,
            })
            .collect()
    }
}

impl<'a, T: std::hash::Hash + Eq + 'static> FromIterator<SavedTick<&'a T>>
    for ChunkTickScheduler<&'a T>
{
    fn from_iter<I: IntoIterator<Item = SavedTick<&'a T>>>(iter: I) -> Self {
        let scheduler = Self::default();
        let ticks: Vec<_> = iter.into_iter().collect();
        // LevelChunkTicks.unpack gives saved ticks negative orders, before fresh ticks.
        let sub_tick_base = -(ticks.len() as i64);
        for (index, tick) in ticks.into_iter().enumerate() {
            scheduler.schedule_saved_tick(&tick, sub_tick_base + index as i64);
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
