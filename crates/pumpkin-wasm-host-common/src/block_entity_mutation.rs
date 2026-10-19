use std::sync::Arc;

use pumpkin_core::{block::entities::BlockEntity, world::World};

use crate::state::PluginHostState;

fn is_registered(world: &World, entity: &Arc<dyn BlockEntity>) -> bool {
    let position = entity.get_position();
    world
        .block_entities
        .get(&position.chunk_position())
        .and_then(|entities| entities.get(&position).cloned())
        .is_some_and(|current| Arc::ptr_eq(&current, entity))
}

/// Keeps a live host resource mutation ahead of its world's final chunk capture.
pub fn with_block_entity_mutation<T: BlockEntity, R>(
    state: &PluginHostState,
    entity: &Arc<T>,
    mutation: impl FnOnce() -> R,
) -> wasmtime::Result<R> {
    let server = state
        .server
        .as_ref()
        .ok_or_else(|| wasmtime::Error::msg("Server context not set"))?;
    let entity: Arc<dyn BlockEntity> = entity.clone();
    let worlds = server.worlds.load_full();
    let world = worlds
        .iter()
        .find(|world| is_registered(world, &entity))
        .ok_or_else(|| wasmtime::Error::msg("Block entity resource is detached or replaced"))?;
    let permit = world
        .block_entity_mutation_permit(entity.get_position())
        .ok_or_else(|| wasmtime::Error::msg("Block entity mutation rejected during unload"))?;
    if !is_registered(world, &entity) {
        return Err(wasmtime::Error::msg(
            "Block entity resource was replaced before mutation admission",
        ));
    }
    // LevelChunk.clearAllBlockEntities waits for admitted mutations; this ticket owns no lock guard.
    let result = mutation();
    drop(permit);
    Ok(result)
}
