//! ChunkMap.scheduleUnload's expected-holder check shared by ordinary saves and retirement.

use std::sync::{Arc, atomic::Ordering};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::{chunk::io::Dirtiable, level::EntityUnload};

use super::{World, entity_persistence, merge_entity_records};
use crate::entity::{EntityBase, RemovalReason};

pub(super) struct EntityChunkSnapshot {
    #[expect(
        clippy::rc_buffer,
        reason = "Preserve World.entities registry incarnation for compare-and-swap"
    )]
    current: Arc<Vec<Arc<dyn EntityBase>>>,
    revisions: Vec<u64>,
    pub(super) members: Vec<Arc<dyn EntityBase>>,
    records: Vec<NbtCompound>,
}

impl EntityChunkSnapshot {
    pub(super) fn prepare(world: &World, position: Vector2<i32>) -> Self {
        crate::entity::persistence_revision::assert_released();
        let current = world.entities.load_full();
        let revisions = current
            .iter()
            .map(|entity| entity.get_entity().persistence_revision.read())
            .collect();
        let members: Vec<_> = current
            .iter()
            .filter(|entity| {
                let base = entity.get_entity();
                std::ptr::eq(base.world.load().as_ref(), world)
                    && entity_persistence::root_chunk(entity) == position
            })
            .cloned()
            .collect();
        let records = members
            .iter()
            .filter(|entity| {
                let base = entity.get_entity();
                !base.is_removed() && base.get_vehicle().is_none()
            })
            .map(entity_persistence::save_riding_tree)
            .collect();
        Self {
            current,
            revisions,
            members,
            records,
        }
    }

    pub(super) fn commit(self, world: &World, request: &EntityUnload, retire: bool) -> bool {
        if retire && request.storage.is_none() && !self.members.is_empty() {
            return false;
        }
        // Resolve trait objects and build the replacement BEFORE taking admission/entity guards.
        let mut bases: Vec<_> = self
            .current
            .iter()
            .zip(&self.revisions)
            .map(|(entity, revision)| (entity.get_entity(), revision))
            .collect();
        bases.sort_unstable_by_key(|(base, _)| base.entity_id);
        let members: Vec<_> = self
            .members
            .iter()
            .map(|entity| entity.get_entity())
            .collect();
        let ids: rustc_hash::FxHashSet<_> = members.iter().map(|base| base.entity_id).collect();
        let remaining: Vec<_> = self
            .current
            .iter()
            .filter(|entity| !ids.contains(&entity.get_entity().entity_id))
            .cloned()
            .collect();
        let commit = || {
            let Some(mut guards) = bases
                .iter()
                .map(|(base, _)| base.persistence_revision.try_lock())
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            if guards
                .iter()
                .zip(&bases)
                .any(|(guard, (_, revision))| **guard != **revision)
                || !Arc::ptr_eq(&world.entities.load_full(), &self.current)
            {
                return false;
            }
            // A transfer callback can save the source before registry cleanup. Keep its last copy.
            if bases
                .iter()
                .any(|(base, _)| !std::ptr::eq(base.world.load().as_ref(), world))
            {
                return !retire;
            }
            // No mutation can pass revision admission between validation and this storage write.
            let mut data = request.storage.as_ref().map(|storage| {
                storage
                    .data
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            });
            if retire {
                let previous = world
                    .entities
                    .compare_and_swap(&self.current, Arc::new(remaining));
                if !Arc::ptr_eq(&previous, &self.current) {
                    return false;
                }
            }
            if let Some(storage) = &request.storage {
                let live = storage.live.load(Ordering::Acquire);
                if live || !self.records.is_empty() {
                    if let Some(data) = data.as_mut() {
                        merge_entity_records(data, live, self.records);
                    }
                    storage.mark_dirty(true);
                }
                if retire {
                    storage.live.store(false, Ordering::Release);
                }
            }
            if retire {
                for (guard, (base, _)) in guards.iter_mut().zip(&bases) {
                    if ids.contains(&base.entity_id) {
                        crate::entity::persistence_revision::set_removed_fields(
                            &base.removal_reason,
                            &base.removed,
                            RemovalReason::UnloadedToChunk,
                            guard,
                        );
                    }
                }
            }
            true
        };
        if retire {
            world.level.commit_entity_unload(request, commit)
        } else {
            world.level.commit_entity_snapshot(request, commit)
        }
    }
}

impl World {
    pub(super) async fn save_entity_snapshots(&self) {
        let mut positions: rustc_hash::FxHashSet<_> = self
            .level
            .live_entity_chunk_positions()
            .into_iter()
            .collect();
        positions.extend(
            self.entities
                .load()
                .iter()
                .map(entity_persistence::root_chunk),
        );
        for position in positions {
            loop {
                let storage = match self.level.get_entity_chunk(position).await {
                    Ok(storage) => storage,
                    Err(error) => {
                        tracing::error!("Entity snapshot cannot load chunk {position:?}: {error}");
                        break;
                    }
                };
                let Some(request) = self.level.entity_snapshot(&storage) else {
                    continue;
                };
                let snapshot = EntityChunkSnapshot::prepare(self, position);
                if snapshot.commit(self, &request, false) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
    }
}
