use std::{cell::RefCell, sync::Arc};

use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::generation::structure::template::BlockPlacer;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::block::entities::block_entity_name;
use crate::world::World;

impl World {
    pub fn clear_synced_block_events_in_box(&self, min: &BlockPos, max: &BlockPos) {
        let mut events = self
            .synced_block_event_queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        events.retain(|event| {
            let pos = event.pos;
            pos.0.x < min.0.x
                || pos.0.x >= max.0.x
                || pos.0.y < min.0.y
                || pos.0.y >= max.0.y
                || pos.0.z < min.0.z
                || pos.0.z >= max.0.z
        });
    }
}

pub struct WorldBlockPlacer<'a> {
    world: &'a Arc<World>,
    pub block_entity_nbts: Vec<NbtCompound>,
    pub changed_positions: Vec<(BlockPos, BlockStateId)>,
    observed: RefCell<FxHashMap<BlockPos, BlockStateId>>,
    rejected_positions: FxHashSet<BlockPos>,
}

impl<'a> WorldBlockPlacer<'a> {
    #[must_use]
    pub const fn new(world: &'a Arc<World>) -> Self {
        Self {
            world,
            block_entity_nbts: Vec::new(),
            changed_positions: Vec::new(),
            observed: RefCell::new(FxHashMap::with_hasher(rustc_hash::FxBuildHasher)),
            rejected_positions: FxHashSet::with_hasher(rustc_hash::FxBuildHasher),
        }
    }

    #[allow(clippy::unused_async)]
    pub fn finalize(&self) {
        for nbt in &self.block_entity_nbts {
            let Some(block_entity) = crate::block::entities::block_entity_from_nbt(nbt) else {
                continue;
            };
            let position = block_entity.get_position();
            let expected = self.observed.borrow().get(&position).copied();
            if !self.rejected_positions.contains(&position)
                && let Some(expected) = expected
            {
                self.world
                    .add_block_entity_from_observed(block_entity, expected);
            }
        }
    }
}

impl BlockPlacer for WorldBlockPlacer<'_> {
    fn get_block_state(&self, pos: &Vector3<i32>) -> BlockStateId {
        let position = BlockPos::new(pos.x, pos.y, pos.z);
        let state = self.world.get_block_state_id(&position);
        *self.observed.borrow_mut().entry(position).or_insert(state)
    }

    fn set_block_state(&mut self, pos: &Vector3<i32>, state: &BlockState) {
        let block_pos = BlockPos::new(pos.x, pos.y, pos.z);
        let Some(mutation) = self.world.block_entity_mutation_permit(block_pos) else {
            self.rejected_positions.insert(block_pos);
            tracing::warn!(
                "Structure block placement rejected while chunk is unloading at {block_pos:?}"
            );
            return;
        };
        let expected = self
            .observed
            .borrow()
            .get(&block_pos)
            .copied()
            .unwrap_or_else(|| self.world.get_block_state_id(&block_pos));
        let retired = {
            let mut loads = self
                .world
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(replaced) = self
                .world
                .write_block_state_if(&block_pos, state.id, |current| current == expected)
            else {
                self.rejected_positions.insert(block_pos);
                return;
            };
            (replaced.to_block() != state.id.to_block()
                && block_entity_name(Block::from_state_id(replaced)).is_some())
            .then(|| {
                self.world
                    .remove_block_entity_under_publication_gate(&block_pos, &mut loads)
            })
        };
        drop(mutation);
        self.observed.borrow_mut().insert(block_pos, state.id);
        self.rejected_positions.remove(&block_pos);
        if let Some(retired) = retired {
            retired.retire_owner();
            // StructureTemplate.placeInWorld drops original contents unless the template
            // installs another block entity, which clears the original silently.
            if block_entity_name(Block::from_state_id(state.id)).is_none() {
                retired.on_block_replaced(self.world, &block_pos);
            }
        }
        self.changed_positions.push((block_pos, state.id));
    }

    fn add_block_entity(&mut self, nbt: NbtCompound) {
        let (Some(x), Some(y), Some(z)) = (nbt.get_int("x"), nbt.get_int("y"), nbt.get_int("z"))
        else {
            return;
        };
        let position = BlockPos::new(x, y, z);
        // StructureTemplate queues NBT even when its terrain write was rejected.
        if self.rejected_positions.contains(&position) {
            return;
        }
        let Some(mutation) = self.world.block_entity_mutation_permit(position) else {
            self.rejected_positions.insert(position);
            tracing::warn!(
                "Structure block entity rejected while chunk is unloading at {position:?}"
            );
            return;
        };
        let expected = self.world.get_block_state_id(&position);
        drop(mutation);
        self.observed
            .borrow_mut()
            .entry(position)
            .or_insert(expected);
        self.block_entity_nbts.push(nbt);
    }
}
