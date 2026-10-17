//! Snapshot live entity records before publication into entity-chunk storage.

use std::sync::Arc;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::chunk::io::Dirtiable;
use rustc_hash::FxHashMap;
use tracing::error;

use super::{World, entity_persistence, merge_entity_records};
use crate::entity::{Entity, EntityBase};

/// Writes a live entity image without allowing a completed retirement to be overwritten by it.
pub(super) async fn save_entities_by_chunk(
    world: &World,
    entities: &[Arc<dyn EntityBase>],
    snapshot_chunks: impl IntoIterator<Item = Vector2<i32>>,
) {
    // PersistentEntitySectionManager.storeChunkSections publishes root records at a tick boundary.
    let mut groups: FxHashMap<Vector2<i32>, Vec<(&Entity, NbtCompound)>> = FxHashMap::default();
    for entity in entities {
        let base_entity = entity.get_entity();
        // Entity.save stores passengers only inside their root record.
        if base_entity.is_removed() || base_entity.get_vehicle().is_some() {
            continue;
        }
        let nbt = entity_persistence::save_riding_tree(entity);
        groups
            .entry(base_entity.chunk_pos.load())
            .or_default()
            .push((base_entity, nbt));
    }
    for pos in snapshot_chunks {
        groups.entry(pos).or_default();
    }

    for (pos, records) in groups {
        let chunk = if records.is_empty() {
            let Some(chunk) = world.level.get_entity_chunk_sync(&pos) else {
                continue;
            };
            chunk
        } else {
            match world.level.get_entity_chunk(pos).await {
                Ok(chunk) => chunk,
                Err(error) => {
                    error!("Entity snapshot cannot load chunk {pos:?}: {error}");
                    continue;
                }
            }
        };

        // PersistentEntitySectionManager.unloadEntity marks roots removed before changing live
        // state under this lock, then publishes its retirement image afterward. A save that wins
        // this lock is overwritten by retirement; a save after retirement omits the stale root.
        let mut data = chunk
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let live = chunk.live.load(std::sync::atomic::Ordering::Relaxed);
        let records: Vec<_> = records
            .into_iter()
            .filter_map(|(entity, nbt)| (!entity.is_removed()).then_some(nbt))
            .collect();
        if !live && records.is_empty() {
            continue;
        }
        merge_entity_records(&mut data, live, records);
        drop(data);
        chunk.mark_dirty(true);
    }
}
