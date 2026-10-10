use super::{
    Block, BlockPos, Dirtiable, Fluid, FxHashSet, Level, Ordering, ScheduledTick, TickData,
    TickPriority, Vector2,
};
use crate::tick::MAX_SAVED_TICK_DELAY;

impl Level {
    pub(super) fn collect_scheduled_ticks(
        &self,
        active: &FxHashSet<Vector2<i32>>,
        ticks: &mut TickData,
    ) {
        // LevelTicks.collectTicks / schedule: queue changes and container membership are atomic.
        let _registration = self
            .scheduled_tick_registration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let positions: Vec<_> = self
            .chunks_with_scheduled_ticks
            .iter()
            .map(|pos| *pos)
            .collect();
        for pos in positions {
            if let Some(chunk) = self.loaded_chunks.get(&pos) {
                if !active.contains(&pos) {
                    continue;
                }
                ticks.block_ticks.append(&mut chunk.block_ticks.step_tick());
                ticks.fluid_ticks.append(&mut chunk.fluid_ticks.step_tick());
                chunk.mark_dirty(true);
                if !chunk.block_ticks.has_ticks() && !chunk.fluid_ticks.has_ticks() {
                    self.chunks_with_scheduled_ticks.remove(&pos);
                }
            } else {
                self.chunks_with_scheduled_ticks.remove(&pos);
            }
        }
    }
    pub fn schedule_block_tick(
        &self,
        block: &Block,
        block_pos: BlockPos,
        delay: u8,
        priority: TickPriority,
    ) {
        self.schedule_block_tick_after(block, block_pos, u32::from(delay), priority);
    }

    /// Schedules a block tick with a delay that can exceed the short tick wheel.
    /// Delays above the signed-int range of vanilla's `SavedTick` are capped at `i32::MAX`.
    pub fn schedule_block_tick_after(
        &self,
        block: &Block,
        block_pos: BlockPos,
        delay: u32,
        priority: TickPriority,
    ) {
        let Some(_mutation) = self.begin_existing_chunk_mutation(block_pos.chunk_position()) else {
            return;
        };
        let _registration = self
            .scheduled_tick_registration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tick_order = self.schedule_tick_counts.fetch_add(1, Ordering::Relaxed);
        let scheduled_tick = ScheduledTick {
            delay: delay.min(MAX_SAVED_TICK_DELAY),
            position: block_pos,
            priority,
            // SAFETY: `block` is a valid reference that outlives this function call for scheduling.
            value: unsafe { &*std::ptr::from_ref::<Block>(block) },
        };

        let chunk_pos = block_pos.chunk_position();
        if self
            .read_chunk_sync(&chunk_pos, |chunk| {
                chunk.block_ticks.schedule_tick(&scheduled_tick, tick_order);
                chunk.mark_dirty(true);
            })
            .is_some()
        {
            self.chunks_with_scheduled_ticks.insert(chunk_pos);
        }
    }
    pub fn schedule_fluid_tick(
        &self,
        fluid: &Fluid,
        block_pos: BlockPos,
        delay: u8,
        priority: TickPriority,
    ) {
        let Some(_mutation) = self.begin_existing_chunk_mutation(block_pos.chunk_position()) else {
            return;
        };
        let _registration = self
            .scheduled_tick_registration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tick_order = self.schedule_tick_counts.fetch_add(1, Ordering::Relaxed);
        let scheduled_tick = ScheduledTick {
            delay: u32::from(delay),
            position: block_pos,
            priority,
            // SAFETY: `fluid` is a valid reference that outlives this function call for scheduling.
            value: unsafe { &*std::ptr::from_ref::<Fluid>(fluid) },
        };

        let chunk_pos = block_pos.chunk_position();
        if self
            .read_chunk_sync(&chunk_pos, |chunk| {
                chunk.fluid_ticks.schedule_tick(&scheduled_tick, tick_order);
                chunk.mark_dirty(true);
            })
            .is_some()
        {
            self.chunks_with_scheduled_ticks.insert(chunk_pos);
        }
    }
}
