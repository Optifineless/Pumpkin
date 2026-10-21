use std::sync::Arc;

use pumpkin_data::{
    Block, BlockState, BlockStateId,
    biome::Biome,
    fluid::{Fluid, FluidState},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    HeightMap,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    sync::{LockClass, Mutex},
};
use pumpkin_world::{
    ProtoChunk,
    chunk::ChunkHeightmapType,
    generation::{
        blender::blending_data::BlendingData, height_limit::HeightLimitView,
        proto_chunk::GenerationCache,
    },
    world::{BlockAccessor, BlockFlags},
};
use rustc_hash::FxHashMap;

use crate::block::entities::block_entity_from_nbt;
use crate::world::World;

// Local observation bookkeeping only; never held during a world read or callback.
enum GenerationObservations {}
impl LockClass for GenerationObservations {
    const NAME: &'static str = "live generation observations";
    const LEVEL: u8 = 100;
}

pub struct WorldGenerationCache {
    world: Arc<World>,
    center: ProtoChunk,
    observed: Mutex<FxHashMap<BlockPos, BlockStateId>, GenerationObservations>,
    overlay: FxHashMap<BlockPos, BlockStateId>,
    block_entities: Vec<NbtCompound>,
}

impl WorldGenerationCache {
    #[must_use]
    pub fn new(world: Arc<World>, pos: &BlockPos) -> Self {
        let generator = world.level.world_gen.load();
        let center = ProtoChunk::new(pos.0.x >> 4, pos.0.z >> 4, &generator);
        Self {
            world,
            center,
            observed: Mutex::new(FxHashMap::default()),
            overlay: FxHashMap::default(),
            block_entities: Vec::new(),
        }
    }

    fn read(&self, pos: &BlockPos) -> BlockStateId {
        if let Some(state) = self.overlay.get(pos) {
            return *state;
        }
        let state = self.world.get_block_state_id(pos);
        *self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(*pos)
            .or_insert(state)
    }

    fn heightmap(&self, heightmap: ChunkHeightmapType, x: i32, z: i32) -> i32 {
        let min_y = self.world.dimension.min_y;
        self.world
            .level
            .loaded_chunks
            .get(&Vector2::new(x >> 4, z >> 4))
            .map_or(min_y, |chunk| {
                chunk
                    .heightmap
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(heightmap, x & 15, z & 15, min_y)
                    + 1
            })
    }

    pub fn apply(self) {
        self.apply_if_observed();
    }

    pub(crate) fn apply_if_observed(self) -> bool {
        let observed = self
            .observed
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let updates = observed
            .iter()
            .map(|(&position, &expected)| {
                let state = self.overlay.get(&position).copied().unwrap_or(expected);
                (position, expected, state, BlockFlags::NOTIFY_ALL)
            })
            .collect::<Vec<_>>();
        if !self.world.set_block_states_from_observed(&updates) {
            return false;
        }
        for nbt in self.block_entities {
            if let Some(block_entity) = block_entity_from_nbt(&nbt)
                && let Some(expected) = self
                    .overlay
                    .get(&block_entity.get_position())
                    .or_else(|| observed.get(&block_entity.get_position()))
                    .copied()
            {
                self.world
                    .add_block_entity_from_observed(block_entity, expected);
            }
        }
        true
    }
}

impl HeightLimitView for WorldGenerationCache {
    fn height(&self) -> u16 {
        self.world.dimension.height as u16
    }

    fn bottom_y(&self) -> i8 {
        self.world.dimension.min_y as i8
    }
}

impl BlockAccessor for WorldGenerationCache {
    fn get_block(&self, position: &BlockPos) -> &'static Block {
        Block::from_state_id(self.read(position))
    }

    fn get_block_state(&self, position: &BlockPos) -> &'static BlockState {
        BlockState::from_id(self.read(position))
    }

    fn get_block_state_id(&self, position: &BlockPos) -> BlockStateId {
        self.read(position)
    }

    fn get_block_and_state(&self, position: &BlockPos) -> (&'static Block, &'static BlockState) {
        BlockState::from_id_with_block(self.read(position))
    }
}

impl GenerationCache for WorldGenerationCache {
    fn get_center_chunk_mut(&mut self) -> &mut ProtoChunk {
        &mut self.center
    }

    fn get_center_chunk(&self) -> &ProtoChunk {
        &self.center
    }

    fn get_chunk_mut(&mut self, _chunk_x: i32, _chunk_z: i32) -> Option<&mut ProtoChunk> {
        None
    }

    fn get_chunk(&self, _chunk_x: i32, _chunk_z: i32) -> Option<&ProtoChunk> {
        None
    }

    fn try_get_proto_chunk(&self, _chunk_x: i32, _chunk_z: i32) -> Option<&ProtoChunk> {
        None
    }

    fn get_block_state(&self, pos: &Vector3<i32>) -> BlockStateId {
        self.read(&BlockPos(*pos))
    }

    fn get_fluid_and_fluid_state(&self, position: &Vector3<i32>) -> (Fluid, FluidState) {
        self.read(&BlockPos(*position));
        let (fluid, state) = self.world.get_fluid_and_fluid_state(&BlockPos(*position));
        (fluid.clone(), state)
    }

    fn set_block_state(&mut self, pos: &Vector3<i32>, block_state: &BlockState) {
        let position = BlockPos(*pos);
        self.read(&position);
        self.overlay.insert(position, block_state.id);
    }

    fn add_block_entity(&mut self, pos: &Vector3<i32>, mut nbt: NbtCompound) {
        self.read(&BlockPos(*pos));
        nbt.put_int("x", pos.x);
        nbt.put_int("y", pos.y);
        nbt.put_int("z", pos.z);
        self.block_entities.push(nbt);
    }

    fn top_motion_blocking_block_height_exclusive(&self, x: i32, z: i32) -> i32 {
        self.heightmap(ChunkHeightmapType::MotionBlocking, x, z)
    }

    fn top_motion_blocking_block_no_leaves_height_exclusive(&self, x: i32, z: i32) -> i32 {
        self.heightmap(ChunkHeightmapType::MotionBlockingNoLeaves, x, z)
    }

    fn get_top_y(&self, heightmap: &HeightMap, x: i32, z: i32) -> i32 {
        match heightmap {
            HeightMap::WorldSurfaceWg
            | HeightMap::WorldSurface
            | HeightMap::OceanFloorWg
            | HeightMap::OceanFloor => self.top_block_height_exclusive(x, z),
            HeightMap::MotionBlocking => self.top_motion_blocking_block_height_exclusive(x, z),
            HeightMap::MotionBlockingNoLeaves => {
                self.top_motion_blocking_block_no_leaves_height_exclusive(x, z)
            }
        }
    }

    fn top_block_height_exclusive(&self, x: i32, z: i32) -> i32 {
        self.heightmap(ChunkHeightmapType::WorldSurface, x, z)
    }

    fn ocean_floor_height_exclusive(&self, x: i32, z: i32) -> i32 {
        self.heightmap(ChunkHeightmapType::WorldSurface, x, z)
    }

    fn is_air(&self, local_pos: &Vector3<i32>) -> bool {
        BlockState::from_id(self.read(&BlockPos(*local_pos))).is_air()
    }

    fn get_biome_for_terrain_gen(&self, x: i32, y: i32, z: i32) -> &'static Biome {
        self.world.get_biome(&BlockPos::new(x, y, z))
    }

    fn get_blending_data(&self, _chunk_x: i32, _chunk_z: i32) -> Option<&BlendingData> {
        None
    }
}
