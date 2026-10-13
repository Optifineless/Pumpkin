use super::{Level, TickData};
use crate::tick::{MAX_SCHEDULED_TICKS_PER_TICK, drain_containers};
use pumpkin_util::math::vector2::Vector2;
use rustc_hash::FxHashSet;
use std::sync::atomic::Ordering;

pub(super) fn collect(
    level: &Level,
    active_chunks: &FxHashSet<Vector2<i32>>,
    ticks: &mut TickData,
) {
    let mut scheduled_chunk_pos = std::mem::take(
        &mut *level
            .scheduled_tick_collect_positions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    scheduled_chunk_pos.clear();
    // Release the index before consulting the loaded registry.
    scheduled_chunk_pos.extend(
        level
            .chunks_with_scheduled_ticks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .copied(),
    );
    let current_tick = level.game_time.load(Ordering::Relaxed);
    let capacity = scheduled_chunk_pos.len().min(active_chunks.len());
    let mut containers = Vec::new();
    for &pos in &scheduled_chunk_pos {
        let empty = if let Some(chunk) = level.loaded_chunks.get(&pos) {
            if !active_chunks.contains(&pos) {
                continue;
            }
            // LevelTicks.sortContainersToTick / nextTickForContainer: future chunks
            // need neither a scheduler lock nor a retained Arc for collection.
            if chunk.block_ticks.has_due_ticks(current_tick)
                || chunk.fluid_ticks.has_due_ticks(current_tick)
            {
                if containers.is_empty() {
                    containers.reserve(capacity);
                }
                containers.push((pos, chunk.value().clone()));
                false
            } else {
                chunk.block_ticks.is_empty_hint() && chunk.fluid_ticks.is_empty_hint()
            }
        } else {
            true
        };
        if empty {
            remove_empty(level, pos, active_chunks);
        }
    }
    scheduled_chunk_pos.clear();
    let mut buffer = level
        .scheduled_tick_collect_positions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if scheduled_chunk_pos.capacity() > buffer.capacity() {
        *buffer = scheduled_chunk_pos;
    }
    drop(buffer);
    if containers.is_empty() {
        return;
    }
    let block_containers: Vec<_> = containers
        .iter()
        .map(|(_, chunk)| &chunk.block_ticks)
        .collect();
    let fluid_containers: Vec<_> = containers
        .iter()
        .map(|(_, chunk)| &chunk.fluid_ticks)
        .collect();
    ticks.block_ticks = drain_containers(
        &block_containers,
        current_tick,
        MAX_SCHEDULED_TICKS_PER_TICK,
    );
    ticks.fluid_ticks = drain_containers(
        &fluid_containers,
        current_tick,
        MAX_SCHEDULED_TICKS_PER_TICK,
    );
    for (pos, chunk) in containers {
        if !chunk.block_ticks.has_ticks() && !chunk.fluid_ticks.has_ticks() {
            remove_empty(level, pos, active_chunks);
        }
    }
}

fn remove_empty(level: &Level, pos: Vector2<i32>, active: &FxHashSet<Vector2<i32>>) {
    #[cfg(test)]
    {
        let hook = level.before_tick_chunk_remove.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }
    level.unregister_empty_tick_container(pos, active);
}
