use std::{cmp::Reverse, collections::BinaryHeap, str::FromStr};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    math::position::BlockPos,
    resource_location::{FromResourceLocation, ResourceLocation, ToResourceLocation},
};

pub mod scheduler;

const MAX_TICK_DELAY: usize = 1 << 8;
/// Largest positive delay representable by vanilla's `SavedTick.codec`.
pub const MAX_SAVED_TICK_DELAY: u32 = i32::MAX as u32;
// ServerLevel.tick passes this independent budget to blockTicks and fluidTicks.
pub const MAX_SCHEDULED_TICKS_PER_TICK: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
#[repr(i32)]
pub enum TickPriority {
    ExtremelyHigh = -3,
    VeryHigh = -2,
    High = -1,
    Normal = 0,
    Low = 1,
    VeryLow = 2,
    ExtremelyLow = 3,
}

impl TickPriority {
    #[must_use]
    pub const fn values() -> [Self; 7] {
        [
            Self::ExtremelyHigh,
            Self::VeryHigh,
            Self::High,
            Self::Normal,
            Self::Low,
            Self::VeryLow,
            Self::ExtremelyLow,
        ]
    }
}

#[derive(Debug)]
pub struct TickPriorityNotFound;

impl TryFrom<i32> for TickPriority {
    type Error = TickPriorityNotFound;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            -3 => Ok(Self::ExtremelyHigh),
            -2 => Ok(Self::VeryHigh),
            -1 => Ok(Self::High),
            0 => Ok(Self::Normal),
            1 => Ok(Self::Low),
            2 => Ok(Self::VeryLow),
            3 => Ok(Self::ExtremelyLow),
            _ => Err(TickPriorityNotFound),
        }
    }
}

#[derive(Clone)]
pub struct ScheduledTick<T> {
    pub delay: u32,
    pub priority: TickPriority,
    pub position: BlockPos,
    pub value: T,
}

/// A chunk tick's signed delay relative to the save or load game time.
#[derive(Clone)]
pub struct SavedTick<T> {
    pub delay: i32,
    pub priority: TickPriority,
    pub position: BlockPos,
    pub value: T,
}

impl<T> From<ScheduledTick<T>> for SavedTick<T> {
    fn from(tick: ScheduledTick<T>) -> Self {
        Self {
            delay: tick.delay.min(MAX_SAVED_TICK_DELAY) as i32,
            priority: tick.priority,
            position: tick.position,
            value: tick.value,
        }
    }
}

#[derive(Clone)]
pub struct OrderedTick<T> {
    pub priority: TickPriority,
    pub sub_tick_order: i64,

    pub position: BlockPos,
    pub value: T,
}

impl<T> OrderedTick<T> {
    pub const fn new(position: BlockPos, value: T) -> Self {
        Self {
            priority: TickPriority::Normal,
            sub_tick_order: 0,
            position,
            value,
        }
    }
}

impl<T> PartialEq for OrderedTick<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sub_tick_order == other.sub_tick_order
    }
}

impl<T> Eq for OrderedTick<T> {}

impl<T> PartialOrd for OrderedTick<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl<T> Ord for OrderedTick<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.sub_tick_order.cmp(&other.sub_tick_order))
    }
}

impl<T> SavedTick<T>
where
    T: ToResourceLocation,
{
    #[must_use]
    pub fn to_nbt_compound(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_int("x", self.position.0.x);
        nbt.put_int("y", self.position.0.y);
        nbt.put_int("z", self.position.0.z);
        nbt.put_int("t", self.delay);
        nbt.put_int("p", self.priority as i32);
        nbt.put_string("i", self.value.to_resource_location());
        nbt
    }
}

impl<T> SavedTick<T>
where
    T: FromResourceLocation,
{
    #[must_use]
    pub fn from_nbt_compound(nbt: &NbtCompound) -> Option<Self> {
        let x = nbt.get_int("x")?;
        let y = nbt.get_int("y")?;
        let z = nbt.get_int("z")?;
        let delay = nbt.get_int("t")?;
        let priority = TickPriority::try_from(nbt.get_int("p")?).ok()?;
        let res_loc_str = nbt.get_string("i")?;
        let res_loc = ResourceLocation::from_str(res_loc_str).ok()?;
        let value = T::from_resource_location(&res_loc)?;

        Some(Self {
            delay,
            priority,
            position: BlockPos::new(x, y, z),
            value,
        })
    }
}

/// Merges due chunk queues by priority and sub-tick order without reordering a chunk's triggers.
pub(crate) fn drain_containers<'a, T: std::hash::Hash + Eq>(
    containers: &[&scheduler::ChunkTickScheduler<&'a T>],
    current_tick: i64,
    max_ticks: usize,
) -> Vec<OrderedTick<&'a T>> {
    // LevelTicks.sortContainersToTick / drainContainers: only due containers enter the heap.
    let prepared = containers
        .iter()
        .enumerate()
        .filter_map(|(index, container)| {
            if !container.has_due_ticks(current_tick) {
                return None;
            }
            container
                .prepare_tick(current_tick)
                .map(|head| (index, head))
        });
    drain_prepared_containers(containers, prepared, max_ticks)
}

/// Drains prepared containers, rechecking their current heads under the scheduler owner.
/// Each prepared index must identify its container in `containers`, at most once.
pub(crate) fn drain_prepared_containers<'a, T: std::hash::Hash + Eq>(
    containers: &[&scheduler::ChunkTickScheduler<&'a T>],
    prepared: impl IntoIterator<Item = (usize, (scheduler::TickKey, usize))>,
    max_ticks: usize,
) -> Vec<OrderedTick<&'a T>> {
    let mut heads = BinaryHeap::new();
    let mut estimated = 0;
    for (index, ((priority, sub_order, trigger), count)) in prepared {
        heads.push(Reverse((priority, sub_order, index, trigger)));
        estimated += count;
    }
    let mut due = Vec::with_capacity(max_ticks.min(estimated));
    while due.len() < max_ticks
        && let Some(Reverse((_, _, index, _))) = heads.pop()
    {
        let next_best = heads
            .peek()
            .map(|Reverse((priority, order, _, _))| (*priority, *order));
        if let Some((priority, order, trigger)) =
            containers[index].poll_due_batch(max_ticks - due.len(), next_best, &mut due)
        {
            heads.push(Reverse((priority, order, index, trigger)));
        }
    }
    due
}

#[cfg(test)]
mod collection_tests;

#[cfg(test)]
mod collection_invariants_tests;
