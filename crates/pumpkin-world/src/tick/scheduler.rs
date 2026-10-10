use std::collections::BTreeMap;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashSet;

use crate::tick::{MAX_SAVED_TICK_DELAY, MAX_TICK_DELAY, OrderedTick, ScheduledTick};

pub struct ChunkTickScheduler<T> {
    inner: Mutex<Option<Box<ChunkTickSchedulerInner<T>>>>,
    offset: AtomicUsize,
}

struct ChunkTickSchedulerInner<T> {
    tick_queue: [Vec<OrderedTick<T>>; MAX_TICK_DELAY],
    queued_ticks: FxHashSet<(BlockPos, T)>,
    long_ticks: BTreeMap<usize, Vec<OrderedTick<T>>>,
}

impl<'a, T: std::hash::Hash + Eq> ChunkTickScheduler<&'a T> {
    pub fn step_tick(&self) -> Vec<OrderedTick<&'a T>> {
        // The offset only changes under `inner`, so `schedule_tick` can't pick a slot that was
        // just drained.
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current_offset = self.offset.fetch_add(1, Ordering::SeqCst);

        let Some(inner) = inner_guard.as_mut() else {
            return Vec::new();
        };

        let mut res = std::mem::take(&mut inner.tick_queue[current_offset % MAX_TICK_DELAY]);
        if let Some(mut due) = inner.long_ticks.remove(&current_offset) {
            res.append(&mut due);
        }

        if !res.is_empty() {
            for next_tick in &res {
                inner
                    .queued_ticks
                    .remove(&(next_tick.position, next_tick.value));
            }
            if inner.queued_ticks.is_empty() {
                *inner_guard = None;
            }
        }
        res
    }

    pub fn schedule_tick(&self, tick: &ScheduledTick<&'a T>, sub_tick_order: i64) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let offset = self.offset.load(Ordering::SeqCst);
        let inner = inner_guard.get_or_insert_with(|| {
            Box::new(ChunkTickSchedulerInner {
                tick_queue: std::array::from_fn(|_| Vec::new()),
                queued_ticks: FxHashSet::default(),
                long_ticks: BTreeMap::new(),
            })
        });

        if inner.queued_ticks.insert((tick.position, tick.value)) {
            // `offset` is the queue the next `step_tick` drains, so a delay of N lands N - 1 slots
            // ahead. Vanilla runs a delay 0 tick on the next tick too.
            // LevelTicks.schedule retains long delays, including DriedGhastBlock's 5000 ticks.
            let delay = tick.delay.min(MAX_SAVED_TICK_DELAY) as usize;
            let due = offset + delay.max(1) - 1;
            let queue = if delay > MAX_TICK_DELAY {
                inner.long_ticks.entry(due).or_default()
            } else {
                &mut inner.tick_queue[due % MAX_TICK_DELAY]
            };
            queue.push(OrderedTick {
                priority: tick.priority,
                sub_tick_order,
                position: tick.position,
                value: tick.value,
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
            queue.retain(|tick| !contains(&tick.position));
        }
        inner.long_ticks.retain(|_, queue| {
            queue.retain(|tick| !contains(&tick.position));
            !queue.is_empty()
        });
        inner
            .queued_ticks
            .retain(|(position, _)| !contains(position));
        let became_empty = inner.queued_ticks.is_empty();

        if became_empty {
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
    pub fn to_vec(&self) -> Vec<ScheduledTick<&'a T>> {
        let inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let offset = self.offset.load(Ordering::SeqCst);
        let Some(inner) = inner_guard.as_ref() else {
            return Vec::new();
        };

        let mut res = Vec::with_capacity(inner.queued_ticks.len());

        for i in 0..MAX_TICK_DELAY {
            let index = (offset + i) % MAX_TICK_DELAY;
            // Inverse of `schedule_tick`: the queue at `offset` runs next tick, i.e. delay 1.
            res.extend(inner.tick_queue[index].iter().map(|x| {
                (
                    x.sub_tick_order,
                    ScheduledTick {
                        delay: (i + 1) as u32,
                        priority: x.priority,
                        position: x.position,
                        value: x.value,
                    },
                )
            }));
        }
        for (due, queue) in &inner.long_ticks {
            res.extend(queue.iter().map(|tick| {
                (
                    tick.sub_tick_order,
                    ScheduledTick {
                        delay: (due - offset + 1) as u32,
                        priority: tick.priority,
                        position: tick.position,
                        value: tick.value,
                    },
                )
            }));
        }
        // LevelChunkTicks.pack saves both queues in sub-tick order, regardless of due time.
        res.sort_by_key(|(order, _)| *order);
        res.into_iter().map(|(_, tick)| tick).collect()
    }
}

impl<'a, T: std::hash::Hash + Eq + 'static> FromIterator<ScheduledTick<&'a T>>
    for ChunkTickScheduler<&'a T>
{
    fn from_iter<I: IntoIterator<Item = ScheduledTick<&'a T>>>(iter: I) -> Self {
        let scheduler = Self::default();
        let ticks: Vec<_> = iter.into_iter().collect();

        let lower = ticks.len();
        if lower > 0 {
            let mut inner_guard = scheduler
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let inner = inner_guard.get_or_insert_with(|| {
                Box::new(ChunkTickSchedulerInner {
                    tick_queue: std::array::from_fn(|_| Vec::new()),
                    queued_ticks: FxHashSet::default(),
                    long_ticks: BTreeMap::new(),
                })
            });
            inner.queued_ticks.reserve(lower);
        }

        // LevelChunkTicks.unpack gives saved ticks distinct negative orders, before fresh ticks.
        let sub_tick_base = -(ticks.len() as i64);
        for (index, tick) in ticks.into_iter().enumerate() {
            scheduler.schedule_tick(&tick, sub_tick_base + index as i64);
        }
        scheduler
    }
}

impl<T> Default for ChunkTickScheduler<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            offset: AtomicUsize::new(0),
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
        assert!(scheduler.step_tick().is_empty());
        assert_eq!(scheduler.step_tick().len(), 1);
    }

    #[test]
    fn saved_delay_round_trips() {
        let scheduler = ChunkTickScheduler::default();
        scheduler.schedule_tick(&tick(5), 0);
        scheduler.step_tick();
        assert_eq!(scheduler.to_vec()[0].delay, 4);
    }

    #[test]
    fn mixed_queue_reload_preserves_order_before_fresh_ticks() {
        let scheduler = ChunkTickScheduler::default();
        let a = BlockPos::new(0, 0, 0);
        let b = BlockPos::new(1, 0, 0);
        let fresh = BlockPos::new(2, 0, 0);
        scheduler.schedule_tick(&tick(5000), 100);
        for _ in 0..4800 {
            assert!(scheduler.step_tick().is_empty());
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
        for _ in 0..199 {
            assert!(scheduler.step_tick().is_empty());
        }
        let mut due = scheduler.step_tick();
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
        for _ in 0..300 {
            assert!(scheduler.step_tick().is_empty());
        }
        let saved = scheduler.to_vec();
        assert_eq!(saved[0].delay, 4700);
        let scheduler: ChunkTickScheduler<_> = saved.into_iter().collect();
        for _ in 0..4699 {
            assert!(scheduler.step_tick().is_empty());
        }
        assert_eq!(scheduler.step_tick().len(), 1);
        assert!(!scheduler.has_ticks());
        scheduler.schedule_tick(&tick(1), 1);
        assert_eq!(scheduler.step_tick().len(), 1);
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
