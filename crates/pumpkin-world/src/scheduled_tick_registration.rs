use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use rustc_hash::FxHashSet;

use crate::level::{Level, TickData};

impl Level {
    pub(crate) fn collect_scheduled_ticks(
        &self,
        active: &FxHashSet<Vector2<i32>>,
        ticks: &mut TickData,
    ) {
        // LevelTicks.collectTicks: scheduling, addContainer and index removal have one owner.
        let mut registered = self
            .chunks_with_scheduled_ticks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registered.retain(|pos| {
            let keep = self.loaded_chunks.get(pos).is_some_and(|chunk| {
                // LevelTicks.tickCheck leaves inactive containers pending.
                if !active.contains(pos) {
                    return true;
                }
                ticks.block_ticks.append(&mut chunk.block_ticks.step_tick());
                ticks.fluid_ticks.append(&mut chunk.fluid_ticks.step_tick());
                chunk.block_ticks.has_ticks() || chunk.fluid_ticks.has_ticks()
            });
            #[cfg(test)]
            if !keep {
                let hook = self.before_tick_unregister.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            keep
        });
    }

    /// Registers the current loaded container's saved ticks under the scheduling lock.
    /// Call after publishing the chunk and releasing chunk-map guards.
    pub fn register_loaded_chunk_ticks(&self, pos: Vector2<i32>) {
        let mut registered = self
            .chunks_with_scheduled_ticks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // LevelTicks.addContainer indexes the saved head before the container can be drained.
        if self
            .read_chunk_sync(&pos, |chunk| {
                chunk.block_ticks.has_ticks() || chunk.fluid_ticks.has_ticks()
            })
            .unwrap_or(false)
        {
            registered.insert(pos);
        }
    }

    /// Clears block ticks in an area without losing concurrent scheduler registration.
    /// `min` is inclusive and `max` is exclusive; empty areas are ignored.
    pub fn clear_scheduled_block_ticks(&self, min: &BlockPos, max: &BlockPos) {
        if max.0.x <= min.0.x || max.0.y <= min.0.y || max.0.z <= min.0.z {
            return;
        }
        let mut registered = self
            .chunks_with_scheduled_ticks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // LevelTicks.clearArea updates the container and its index together.
        for x in min.0.x >> 4..=(max.0.x - 1) >> 4 {
            for z in min.0.z >> 4..=(max.0.z - 1) >> 4 {
                let pos = Vector2::new(x, z);
                if self
                    .read_chunk_sync(&pos, |chunk| {
                        chunk.block_ticks.clear_area(min, max);
                        !chunk.block_ticks.has_ticks() && !chunk.fluid_ticks.has_ticks()
                    })
                    .unwrap_or(false)
                {
                    registered.remove(&pos);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "scheduled_tick_tests.rs"]
mod tests;
