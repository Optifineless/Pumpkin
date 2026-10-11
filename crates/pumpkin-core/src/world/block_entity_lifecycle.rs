use std::sync::Arc;

use pumpkin_data::Block;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{codec::var_int::VarInt, java::client::play::CBlockEntityData};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{chunk::io::Dirtiable, level::SyncChunk};

use super::{World, remove_pending_block_entity};
use crate::block::entities::{BlockEntity, block_entity_from_nbt, block_owns_block_entity};

impl World {
    /// Admits custom data only while terrain remains loaded, including retained lifecycle cells.
    pub(super) fn begin_custom_data_mutation(
        &self,
        pos: &BlockPos,
    ) -> Option<pumpkin_world::level::chunk_lifecycle::ChunkMutation> {
        // Level.setBlock refuses an absent chunk before applying its side effects.
        let chunk_pos = pos.chunk_position();
        if !self.level.is_chunk_loaded(&chunk_pos) {
            return None;
        }
        let mutation = self.level.begin_existing_chunk_mutation(chunk_pos)?;
        self.level.is_chunk_loaded(&chunk_pos).then_some(mutation)
    }

    pub fn get_block_entity(&self, pos: &BlockPos) -> Option<Arc<dyn BlockEntity>> {
        // LevelChunk.getBlockEntity / promotePendingBlockEntity: one publication per incarnation.
        let chunk_pos = pos.chunk_position();
        #[cfg(test)]
        if self.level.chunk_lifecycles.benchmark_admission_disabled() {
            let entity = self
                .block_entities
                .get(&chunk_pos)
                .and_then(|entities| entities.get(pos).cloned())?;
            self.bind_block_entity_context(entity.as_ref());
            return Some(entity);
        }
        // The counter covers lookup through Arc publication; retained users defer the snapshot.
        let _lookup = self.level.try_chunk_mutation(chunk_pos)?;
        if let Some(entity) = self
            .block_entities
            .get(&chunk_pos)
            .and_then(|entities| entities.get(pos).cloned())
        {
            self.bind_block_entity_context(entity.as_ref());
            return Some(entity);
        }
        let chunk = self.level.loaded_chunks.get(&chunk_pos)?.clone();
        // LevelChunk.promotePendingBlockEntity: air probes have nothing to publish.
        if !self.level.chunk_lifecycles.benchmark_legacy_admission()
            && !chunk
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(pos)
        {
            return None;
        }
        let lifecycle = self.level.chunk_lifecycles.at(chunk_pos);
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entity) = self
            .block_entities
            .get(&chunk_pos)
            .and_then(|entities| entities.get(pos).cloned())
        {
            self.bind_block_entity_context(entity.as_ref());
            return Some(entity);
        }
        let relative = pos.chunk_relative_position();
        let block = Block::from_state_id(chunk.section.get_block_absolute_y(
            relative.x as usize,
            relative.y,
            relative.z as usize,
        )?);
        let nbt = chunk
            .pending_block_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(pos)
            .cloned()?;
        if !block_owns_block_entity(block, nbt.get_string("id")?) {
            return None;
        }
        let entity = block_entity_from_nbt(&nbt)?;
        self.bind_block_entity_context(entity.as_ref());
        if let Some(custom) = nbt
            .get_compound("PumpkinCustomData")
            .or_else(|| nbt.get_compound("BukkitValues"))
        {
            self.custom_block_entity_data.insert(*pos, custom.clone());
        }
        state.live_block_entities = true;
        Some(
            self.block_entities
                .entry(chunk_pos)
                .or_default()
                .entry(*pos)
                .or_insert(entity)
                .clone(),
        )
    }

    fn snapshot_block_entities(&self, chunk: &SyncChunk) {
        // LevelChunk.getBlockEntityNbtForSaving: the destination is the retained object.
        let pos = Vector2::new(chunk.x, chunk.z);
        let entities: Vec<_> = self
            .block_entities
            .get(&pos)
            .map(|entities| entities.values().cloned().collect())
            .unwrap_or_default();
        for entity in entities {
            self.snapshot_block_entity(chunk, entity.as_ref());
        }
    }

    fn snapshot_block_entity(&self, chunk: &SyncChunk, entity: &dyn BlockEntity) {
        let mut nbt = NbtCompound::new();
        entity.write_internal(&mut nbt);
        if let Some(custom) = self.custom_block_entity_data.get(&entity.get_position())
            && !custom.is_empty()
        {
            nbt.put_compound("PumpkinCustomData", custom.clone());
        }
        store_snapshot(chunk, entity.get_position(), nbt);
    }

    pub(super) fn save_block_entities(&self, pos: Vector2<i32>) {
        let lifecycle = self.level.chunk_lifecycles.at(pos);
        let _state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(chunk) = self
            .level
            .loaded_chunks
            .get(&pos)
            .map(|chunk| chunk.clone())
        else {
            tracing::error!(
                ?pos,
                "Cannot snapshot block entities without their terrain chunk"
            );
            return;
        };
        self.snapshot_block_entities(&chunk);
    }

    pub(super) fn prepare_block_entity_unload(&self, chunk: &SyncChunk, generation: u64) -> bool {
        // World ticks own the read side. Never wait for it while holding lifecycle ownership.
        let Ok(_ticks) = self.chunk_lifecycle_tick.try_write() else {
            self.queue_chunk_unload(chunk);
            return false;
        };
        let pos = Vector2::new(chunk.x, chunk.z);
        if self.block_entities.get(&pos).is_some_and(|entities| {
            entities
                .values()
                .any(|entity| Arc::strong_count(entity) != 1)
        }) {
            return false;
        }
        if !self.snapshot_unloading_entities(pos, generation) {
            return false;
        }
        self.snapshot_block_entities(chunk);
        true
    }

    pub(super) fn finish_block_entity_unload(&self, chunk: &SyncChunk, generation: u64) -> bool {
        let Ok(_ticks) = self.chunk_lifecycle_tick.try_write() else {
            self.queue_chunk_unload(chunk);
            return false;
        };
        let pos = Vector2::new(chunk.x, chunk.z);
        if !self.detach_unloading_entities(pos, generation) {
            return false;
        }
        self.block_entities.remove(&pos);
        self.detach_unloaded_custom_data(pos);
        true
    }

    pub fn add_block_entity_nbt(&self, pos: BlockPos, nbt: &NbtCompound) {
        let Some(_mutation) = self
            .level
            .begin_existing_chunk_mutation(pos.chunk_position())
        else {
            return;
        };
        let lifecycle = self.level.chunk_lifecycles.at(pos.chunk_position());
        let _state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(chunk) = self.level.loaded_chunks.get(&pos.chunk_position()) else {
            tracing::error!(
                ?pos,
                "Cannot save block entity NBT without its terrain chunk"
            );
            return;
        };
        store_snapshot(&chunk, pos, nbt.clone());
        self.pending_block_entity_migrations
            .push(pos.chunk_position());
    }

    pub fn remove_block_entity(&self, pos: &BlockPos) {
        // LevelChunk.removeBlockEntity shares publication ownership with lazy promotion.
        let chunk_pos = pos.chunk_position();
        let Some(_mutation) = self.level.begin_existing_chunk_mutation(chunk_pos) else {
            return;
        };
        let lifecycle = self.level.chunk_lifecycles.at(chunk_pos);
        let mut state = lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let removed = self
            .block_entities
            .get_mut(&chunk_pos)
            .is_some_and(|mut entities| entities.remove(pos).is_some());
        self.level.read_chunk_sync(&chunk_pos, |chunk| {
            remove_pending_block_entity(chunk, pos);
            if removed {
                chunk.mark_dirty(true);
            }
        });
        self.custom_block_entity_data.remove(pos);
        self.block_entities
            .remove_if(&chunk_pos, |_, entities| entities.is_empty());
        state.live_block_entities = self.block_entities.contains_key(&chunk_pos);
    }
    pub fn add_block_entity(&self, entity: Arc<dyn BlockEntity>) {
        // LevelChunk.setBlockEntity: explicit publication creates admission before live state.
        let pos = entity.get_position();
        let _mutation = self.level.begin_chunk_mutation(pos.chunk_position());
        self.bind_block_entity_context(entity.as_ref());
        let lifecycle = self.level.chunk_lifecycles.at(pos.chunk_position());
        let notification = entity.clone();
        {
            let mut state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.live_block_entities = true;
            self.block_entities
                .entry(pos.chunk_position())
                .or_default()
                .insert(pos, entity);
            if let Some(chunk) = self
                .level
                .loaded_chunks
                .get(&pos.chunk_position())
                .map(|chunk| chunk.clone())
            {
                self.snapshot_block_entity(&chunk, notification.as_ref());
            } else {
                tracing::error!(
                    ?pos,
                    "Cannot snapshot block entity without its terrain chunk"
                );
            }
        };
        self.send_block_entity_update(&notification);
    }

    pub fn update_block_entity(&self, entity: &Arc<dyn BlockEntity>) {
        let pos = entity.get_position();
        let Some(_mutation) = self
            .level
            .begin_existing_chunk_mutation(pos.chunk_position())
        else {
            return;
        };
        let lifecycle = self.level.chunk_lifecycles.at(pos.chunk_position());
        {
            let _state = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !self
                .block_entities
                .get(&pos.chunk_position())
                .and_then(|entities| entities.get(&pos).cloned())
                .is_some_and(|current| Arc::ptr_eq(&current, entity))
            {
                return;
            }
            let Some(chunk) = self
                .level
                .loaded_chunks
                .get(&pos.chunk_position())
                .map(|chunk| chunk.clone())
            else {
                tracing::error!(?pos, "Cannot update block entity without its terrain chunk");
                return;
            };
            self.snapshot_block_entity(&chunk, entity.as_ref());
        };
        self.send_block_entity_update(entity);
    }

    fn send_block_entity_update(&self, entity: &Arc<dyn BlockEntity>) {
        if let Some(nbt) = entity.chunk_data_nbt() {
            let bytes = pumpkin_nbt::Nbt::from(nbt).write_unnamed();
            self.broadcast_to_chunk(
                entity.get_position().chunk_position(),
                &CBlockEntityData::new(
                    entity.get_position(),
                    VarInt(entity.get_id() as i32),
                    bytes.as_ref().into(),
                ),
            );
        }
    }
}

fn store_snapshot(chunk: &SyncChunk, pos: BlockPos, nbt: NbtCompound) {
    chunk
        .pending_block_entities
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(pos, nbt);
    chunk.mark_dirty(true);
}
