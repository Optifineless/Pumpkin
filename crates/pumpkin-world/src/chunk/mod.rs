mod entity_snapshot;
mod random_tick_membership;
use crate::chunk::format::LightContainer;
use crate::tick::scheduler::ChunkTickScheduler;
use palette::{BiomePalette, BlockPalette, has_random_ticking_fluid};
use pumpkin_data::block_properties::{blocks_movement, has_random_ticks, is_air};
use pumpkin_data::chunk::ChunkStatus;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::tag::Block::MINECRAFT_LEAVES;
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use random_tick_membership::RandomTickMembership;
use rustc_hash::{FxHashMap, FxHashSet};

use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use thiserror::Error;

pub mod format;
pub mod io;
pub mod palette;

// TODO
pub const CHUNK_WIDTH: usize = BlockPalette::SIZE;
pub const CHUNK_AREA: usize = CHUNK_WIDTH * CHUNK_WIDTH;
pub const BIOME_VOLUME: usize = BiomePalette::VOLUME;
pub const SUBCHUNK_VOLUME: usize = CHUNK_AREA * CHUNK_WIDTH;

#[derive(Error, Debug)]
pub enum ChunkReadingError {
    #[error("Io error: {0}")]
    IoError(std::io::Error),
    #[error("Invalid header")]
    InvalidHeader,
    #[error("Region is invalid")]
    RegionIsInvalid,
    #[error("Compression error {0}")]
    Compression(CompressionError),
    #[error("Tried to read chunk which does not exist")]
    ChunkNotExist,
    #[error("Failed to parse chunk from bytes: {0}")]
    ParsingError(ChunkParsingError),
}

#[derive(Error, Debug)]
pub enum ChunkWritingError {
    #[error("Io error: {0}")]
    IoError(std::io::Error),
    #[error("Compression error {0}")]
    Compression(CompressionError),
    #[error("Chunk serializing error: {0}")]
    ChunkSerializingError(String),
}

#[derive(Error, Debug)]
pub enum CompressionError {
    #[error("Compression scheme not recognised")]
    UnknownCompression,
    #[error("Error while working with zlib compression: {0}")]
    ZlibError(std::io::Error),
    #[error("Error while working with Gzip compression: {0}")]
    GZipError(std::io::Error),
    #[error("Error while working with LZ4 compression: {0}")]
    LZ4Error(std::io::Error),
    #[error("Error while working with zstd compression: {0}")]
    ZstdError(std::io::Error),
}

// Clone here cause we want to clone a snapshot of the chunk so we don't block writing for too long
pub struct ChunkData {
    pub section: ChunkSections,
    /// See `https://minecraft.wiki/w/Heightmap` for more info
    pub heightmap: std::sync::Mutex<ChunkHeightmaps>,
    pub x: i32,
    pub z: i32,
    pub block_ticks: ChunkTickScheduler<&'static Block>,
    pub fluid_ticks: ChunkTickScheduler<&'static Fluid>,
    pub pending_block_entities: std::sync::Mutex<FxHashMap<BlockPos, NbtCompound>>,
    pub light_engine: std::sync::Mutex<ChunkLight>,
    pub light_populated: AtomicBool,
    // force repair when saved lighting was discarded.
    pub lighting_invalid: AtomicBool,
    pub status: ChunkStatus,
    pub blending_data: Option<crate::generation::blender::blending_data::BlendingData>,
    pub dirty: io::DirtyFlag,
    pub inhabited_time: AtomicU64,
    pub custom_data: std::sync::Mutex<NbtCompound>,
}

pub struct ChunkEntityData {
    /// Chunk X
    pub x: i32,
    /// Chunk Z
    pub z: i32,
    pub data: std::sync::Mutex<Vec<NbtCompound>>,
    /// Untouched storage records, separate from replaceable snapshots of live entities.
    pub dormant_records: std::sync::Mutex<Option<Vec<NbtCompound>>>,
    /// Set once the serialized entities have been copied for spawning. From then on the
    /// live entity list is the source of truth and `data` is rebuilt from it on every save.
    pub live: AtomicBool,

    pub dirty: io::DirtyFlag,
}

impl ChunkEntityData {
    /// Returns saved entities once for activation without consuming a pending save.
    pub fn entities_for_activation(&self) -> Vec<NbtCompound> {
        // PersistentEntitySectionManager.processPendingLoads consumes copied storage NBT.
        let data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.live.swap(true, std::sync::atomic::Ordering::AcqRel) {
            Vec::new()
        } else {
            self.dormant_records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| data.clone())
        }
    }
}

/// Represents pure block data for a chunk.
/// Subchunks are vertical portions of a chunk. They are 16 blocks tall.
/// There are currently 24 subchunks per chunk.
///
/// A chunk can be:
/// - Subchunks: 24 separate subchunks are stored.
pub struct ChunkSections {
    pub count: usize,
    pub block_sections: RwLock<Box<[BlockPalette]>>,
    pub random_tick_sections: RwLock<Option<Box<[RandomTickSectionCache]>>>,
    pub randomly_ticking_mask: RandomTickMembership,
    pub biome_sections: RwLock<Box<[BiomePalette]>>,
    pub min_y: i32,
}

#[derive(Default, Clone, Copy)]
pub struct RandomTickSectionCache {
    pub random_ticking_block_count: u16,
    pub random_ticking_fluid_count: u16,
}

impl RandomTickSectionCache {
    #[must_use]
    pub const fn is_randomly_ticking(&self) -> bool {
        self.random_ticking_block_count > 0 || self.random_ticking_fluid_count > 0
    }
}

impl ChunkSections {
    #[cfg(test)]
    #[must_use]
    pub fn dump_blocks(&self) -> Vec<BlockStateId> {
        self.block_sections
            .read()
            .unwrap()
            .iter()
            .flat_map(|section| section.iter())
            .collect()
    }

    #[cfg(test)]
    #[must_use]
    pub fn dump_biomes(&self) -> Vec<u8> {
        self.biome_sections
            .read()
            .unwrap()
            .iter()
            .flat_map(|section| section.iter())
            .collect()
    }
}

#[derive(Default, Clone)]
pub struct ChunkLight {
    pub sky_light: Box<[LightContainer]>,
    pub block_light: Box<[LightContainer]>,
}

#[derive(Debug, Clone, Copy)]
pub enum ChunkHeightmapType {
    WorldSurface = 0,
    MotionBlocking = 1,
    MotionBlockingNoLeaves = 2,
}
impl TryFrom<usize> for ChunkHeightmapType {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::WorldSurface),
            1 => Ok(Self::MotionBlocking),
            2 => Ok(Self::MotionBlockingNoLeaves),
            _ => Err("Invalid usize value for ChunkHeightmapType. The value should be 0~2."),
        }
    }
}

impl ChunkHeightmapType {
    #[must_use]
    pub fn is_opaque(&self, block_state: &BlockState) -> bool {
        let block = block_state.id.to_block_id();
        match self {
            Self::WorldSurface => !block_state.is_air(),
            Self::MotionBlocking => blocks_movement(block_state, block) || block_state.is_liquid(),
            Self::MotionBlockingNoLeaves => {
                (blocks_movement(block_state, block) || block_state.is_liquid())
                    && !block.has_tag(MINECRAFT_LEAVES)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkHeightmaps {
    // Heightmap uses ceil(log2(dimension height + 1)).
    pub height_bits: u32,
    pub world_surface: Option<Box<[i64]>>,
    pub motion_blocking: Option<Box<[i64]>>,
    pub motion_blocking_no_leaves: Option<Box<[i64]>>,
}

impl ChunkHeightmaps {
    // Heightmap constructor and SimpleBitStorage layout.
    #[must_use]
    pub const fn new(height: i32) -> Self {
        Self {
            height_bits: (height as u32 + 1).next_power_of_two().trailing_zeros(),
            world_surface: None,
            motion_blocking: None,
            motion_blocking_no_leaves: None,
        }
    }

    #[must_use]
    pub const fn longs(&self) -> usize {
        256usize.div_ceil(64 / self.height_bits as usize)
    }

    pub fn set(&mut self, heightmap: ChunkHeightmapType, x: i32, z: i32, height: i32, min_y: i32) {
        let data = match heightmap {
            ChunkHeightmapType::WorldSurface => &mut self.world_surface,
            ChunkHeightmapType::MotionBlocking => &mut self.motion_blocking,
            ChunkHeightmapType::MotionBlockingNoLeaves => &mut self.motion_blocking_no_leaves,
        };
        let values_per_long = 64 / self.height_bits as usize;
        let longs = 256usize.div_ceil(values_per_long);
        let data = data.get_or_insert_with(|| vec![0; longs].into_boxed_slice());

        let local_x = (x & 15) as usize;
        let local_z = (z & 15) as usize;
        let column_idx = local_z * 16 + local_x;

        // In Minecraft 1.16+, height is stored as (y - min_y + 1). 0 means below min_y.
        // values never cross long boundaries.
        let val = (height - min_y + 1).max(0) as u64;
        let array_idx = column_idx / values_per_long;
        let shift = (column_idx % values_per_long) * self.height_bits as usize;

        let value_mask = (1u64 << self.height_bits) - 1;
        let mask = value_mask << shift;

        let mut current = data[array_idx] as u64;
        current = (current & !mask) | ((val & value_mask) << shift);
        data[array_idx] = current as i64;
    }

    #[must_use]
    pub fn get(&self, heightmap: ChunkHeightmapType, x: i32, z: i32, min_y: i32) -> i32 {
        let data = match heightmap {
            ChunkHeightmapType::WorldSurface => &self.world_surface,
            ChunkHeightmapType::MotionBlocking => &self.motion_blocking,
            ChunkHeightmapType::MotionBlockingNoLeaves => &self.motion_blocking_no_leaves,
        };

        let Some(data) = data else {
            return min_y - 1;
        };

        let local_x = (x & 15) as usize;
        let local_z = (z & 15) as usize;
        let column_idx = local_z * 16 + local_x;
        let values_per_long = 64 / self.height_bits as usize;
        let array_idx = column_idx / values_per_long;
        let shift = (column_idx % values_per_long) * self.height_bits as usize;

        let current = data[array_idx] as u64;
        let val = (current >> shift) & ((1u64 << self.height_bits) - 1);

        (val as i32) + min_y - 1
    }

    #[expect(clippy::too_many_arguments)]
    pub fn update<F>(
        &mut self,
        heightmap_type: ChunkHeightmapType,
        local_x: i32,
        local_y: i32,
        local_z: i32,
        block_state: &BlockState,
        min_y: i32,
        get_block: F,
    ) -> bool
    where
        F: Fn(i32) -> &'static BlockState,
    {
        let first_available = self.get(heightmap_type, local_x, local_z, min_y) + 1;
        if local_y <= first_available - 2 {
            return false;
        }

        if heightmap_type.is_opaque(block_state) {
            if local_y >= first_available {
                self.set(heightmap_type, local_x, local_z, local_y, min_y);
                return true;
            }
        } else if first_available - 1 == local_y {
            for y in (min_y..local_y).rev() {
                let state = get_block(y);
                if heightmap_type.is_opaque(state) {
                    self.set(heightmap_type, local_x, local_z, y, min_y);
                    return true;
                }
            }
            self.set(heightmap_type, local_x, local_z, min_y - 1, min_y);
            return true;
        }

        false
    }
}

/// The Heightmap for a completely empty chunk
// only legacy empty/test chunks use the default dimension.
impl Default for ChunkHeightmaps {
    fn default() -> Self {
        Self::new(pumpkin_data::dimension::Dimension::OVERWORLD.height)
    }
}

impl ChunkSections {
    #[must_use]
    pub fn build_random_tick_sections_cache(
        block_sections: &[BlockPalette],
    ) -> (Option<Box<[RandomTickSectionCache]>>, Vec<bool>) {
        let mut mask = vec![false; block_sections.len()];
        let mut has_ticks = false;
        let cache = block_sections
            .iter()
            .enumerate()
            .map(|(i, section)| {
                let (random_ticking_block_count, random_ticking_fluid_count) =
                    section.random_ticking_counts();
                if random_ticking_block_count > 0 || random_ticking_fluid_count > 0 {
                    mask[i] = true;
                    has_ticks = true;
                }
                RandomTickSectionCache {
                    random_ticking_block_count,
                    random_ticking_fluid_count,
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        if has_ticks {
            (Some(cache), mask)
        } else {
            (None, mask)
        }
    }

    #[must_use]
    pub fn new(num_sections: usize, min_y: i32) -> Self {
        let block_sections = vec![BlockPalette::default(); num_sections].into_boxed_slice();
        let (random_tick_sections, randomly_ticking_mask) =
            Self::build_random_tick_sections_cache(&block_sections);
        let biome_sections = vec![BiomePalette::default(); num_sections].into_boxed_slice();

        Self {
            count: num_sections,
            block_sections: RwLock::new(block_sections),
            random_tick_sections: RwLock::new(random_tick_sections),
            randomly_ticking_mask: RandomTickMembership::new(randomly_ticking_mask),
            biome_sections: RwLock::new(biome_sections),
            min_y,
        }
    }

    #[must_use]
    pub(crate) fn from_palettes(
        block_sections: Box<[BlockPalette]>,
        biome_sections: Box<[BiomePalette]>,
        min_y: i32,
    ) -> Self {
        assert_eq!(
            block_sections.len(),
            biome_sections.len(),
            "block and biome section counts must match"
        );
        let count = block_sections.len();
        let (random_tick_sections, randomly_ticking_mask) =
            Self::build_random_tick_sections_cache(&block_sections);

        Self {
            count,
            block_sections: RwLock::new(block_sections),
            random_tick_sections: RwLock::new(random_tick_sections),
            randomly_ticking_mask: RandomTickMembership::new(randomly_ticking_mask),
            biome_sections: RwLock::new(biome_sections),
            min_y,
        }
    }

    #[must_use]
    pub fn get_block_absolute_y(
        &self,
        relative_x: usize,
        y: i32,
        relative_z: usize,
    ) -> Option<BlockStateId> {
        let y = y - self.min_y;
        if y < 0 {
            None
        } else {
            let relative_y = y as usize;
            self.get_relative_block(relative_x, relative_y, relative_z)
        }
    }

    pub fn set_block_absolute_y(
        &self,
        relative_x: usize,
        y: i32,
        relative_z: usize,
        block_state_id: BlockStateId,
    ) -> BlockStateId {
        let y = y - self.min_y;
        if y < 0 {
            return Block::AIR.default_state.id;
        }
        let relative_y = y as usize;
        self.set_block_no_heightmap_update(relative_x, relative_y, relative_z, block_state_id)
    }

    #[must_use]
    pub fn get_rough_biome_absolute_y(
        &self,
        relative_x: usize,
        y: i32,
        relative_z: usize,
    ) -> Option<u8> {
        let y = y - self.min_y;
        if y < 0 {
            None
        } else {
            let relative_y = y as usize;
            self.get_noise_biome(
                relative_y / BlockPalette::SIZE,
                relative_x >> 2 & 3,
                relative_y >> 2 & 3,
                relative_z >> 2 & 3,
            )
        }
    }

    /// Gets the given block in the chunk
    fn get_relative_block(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
    ) -> Option<BlockStateId> {
        debug_assert!(relative_x < BlockPalette::SIZE);
        debug_assert!(relative_z < BlockPalette::SIZE);

        let section_index = relative_y / BlockPalette::SIZE;
        let relative_y = relative_y % BlockPalette::SIZE;
        self.block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(section_index)
            .map(|section| section.get(relative_x, relative_y, relative_z))
    }

    /// Sets the given block in the chunk, returning the old block state ID
    #[inline]
    pub fn set_relative_block(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        block_state_id: BlockStateId,
    ) -> BlockStateId {
        self.set_block_no_heightmap_update(relative_x, relative_y, relative_z, block_state_id)
    }

    /// Sets the given block in the chunk, returning the old block
    /// Contrary to `set_block` this does not update the heightmap.
    ///
    /// Only use this if you know you don't need to update the heightmap
    /// or if you manually set the heightmap in `empty_with_heightmap`
    pub fn set_block_no_heightmap_update(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        block_state_id: BlockStateId,
    ) -> BlockStateId {
        self.set_block_if_no_heightmap_update(
            relative_x,
            relative_y,
            relative_z,
            block_state_id,
            |_| true,
        )
        .unwrap_or(BlockStateId::AIR)
    }

    /// Like `set_block_no_heightmap_update`, but only sets the block when `condition` accepts
    /// the current state. The check and the write share one lock, so no other write can land
    /// in between. Returns the replaced state, or `None` when nothing was written.
    pub fn set_block_if_no_heightmap_update(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        block_state_id: BlockStateId,
        condition: impl FnOnce(BlockStateId) -> bool,
    ) -> Option<BlockStateId> {
        debug_assert!(relative_x < BlockPalette::SIZE);
        debug_assert!(relative_z < BlockPalette::SIZE);

        let section_index = relative_y / BlockPalette::SIZE;
        let relative_y = relative_y % BlockPalette::SIZE;

        // Keep lock order consistent to avoid deadlocks: block sections first, then random-tick cache.
        let mut sections = self
            .block_sections
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut random_tick_sections_guard = self
            .random_tick_sections
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(section) = sections.get_mut(section_index) {
            if !condition(section.get(relative_x, relative_y, relative_z)) {
                return None;
            }
            let replaced_block_state_id =
                section.set(relative_x, relative_y, relative_z, block_state_id);
            if replaced_block_state_id == block_state_id {
                return Some(replaced_block_state_id);
            }

            if (has_random_ticks(block_state_id) || has_random_ticking_fluid(block_state_id))
                && random_tick_sections_guard.is_none()
            {
                let new_cache =
                    vec![RandomTickSectionCache::default(); self.count].into_boxed_slice();
                *random_tick_sections_guard = Some(new_cache);
            }

            if let Some(random_tick_sections) = random_tick_sections_guard.as_mut() {
                let random_tick_cache = &mut random_tick_sections[section_index];
                if has_random_ticks(replaced_block_state_id) {
                    random_tick_cache.random_ticking_block_count = random_tick_cache
                        .random_ticking_block_count
                        .saturating_sub(1);
                }
                if has_random_ticking_fluid(replaced_block_state_id) {
                    random_tick_cache.random_ticking_fluid_count = random_tick_cache
                        .random_ticking_fluid_count
                        .saturating_sub(1);
                }

                if has_random_ticks(block_state_id) {
                    random_tick_cache.random_ticking_block_count = random_tick_cache
                        .random_ticking_block_count
                        .saturating_add(1);
                }
                if has_random_ticking_fluid(block_state_id) {
                    random_tick_cache.random_ticking_fluid_count = random_tick_cache
                        .random_ticking_fluid_count
                        .saturating_add(1);
                }

                self.randomly_ticking_mask
                    .set(section_index, random_tick_cache.is_randomly_ticking());
            }

            return Some(replaced_block_state_id);
        }
        None
    }

    pub fn set_relative_biome(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        biome_id: u8,
    ) {
        debug_assert!(relative_x < BiomePalette::SIZE);
        debug_assert!(relative_z < BiomePalette::SIZE);

        let section_index = relative_y / BiomePalette::SIZE;
        let relative_y = relative_y % BiomePalette::SIZE;
        if let Some(section) = self
            .biome_sections
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(section_index)
        {
            section.set(relative_x, relative_y, relative_z, biome_id);
        }
    }

    #[must_use]
    pub fn get_noise_biome(
        &self,
        index: usize,
        scale_x: usize,
        scale_y: usize,
        scale_z: usize,
    ) -> Option<u8> {
        debug_assert!(scale_x < BiomePalette::SIZE);
        debug_assert!(scale_z < BiomePalette::SIZE);
        self.biome_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(index)
            .map(|section| section.get(scale_x, scale_y, scale_z))
    }

    #[must_use]
    pub fn get_top_y(&self, relative_x: usize, relative_z: usize, first_y: i32) -> Option<i32> {
        debug_assert!(relative_x < BlockPalette::SIZE);
        debug_assert!(relative_z < BlockPalette::SIZE);

        let mut y = first_y;
        while y >= self.min_y {
            if let Some(block_state_id) = self.get_block_absolute_y(relative_x, y, relative_z)
                && !is_air(block_state_id)
            {
                return Some(y);
            }
            y -= 1;
        }
        None
    }
}

impl ChunkData {
    /// Validates and primes saved heightmaps before the loaded chunk becomes visible.
    pub(crate) fn prime_heightmaps(&self, dimension_height: i32) {
        let mut maps = self
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let layout = ChunkHeightmaps::new(dimension_height);
        let longs = layout.longs();
        maps.height_bits = layout.height_bits;
        let repair = [
            maps.world_surface
                .as_ref()
                .is_none_or(|data| data.len() != longs),
            maps.motion_blocking
                .as_ref()
                .is_none_or(|data| data.len() != longs),
            maps.motion_blocking_no_leaves
                .as_ref()
                .is_none_or(|data| data.len() != longs),
        ];
        if !repair.iter().any(|invalid| *invalid) {
            return;
        }
        // Heightmap.primeHeightmaps scans only the missing types, preserving valid saved maps.
        let mut primed = layout;
        let highest = self.get_highest_non_empty_subchunk();
        for x in 0..16 {
            for z in 0..16 {
                self.populate_heightmaps(&mut primed, highest, x, z);
            }
        }
        if repair[0] {
            maps.world_surface = primed.world_surface;
        }
        if repair[1] {
            maps.motion_blocking = primed.motion_blocking;
        }
        if repair[2] {
            maps.motion_blocking_no_leaves = primed.motion_blocking_no_leaves;
        }
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[must_use]
    pub fn empty(x: i32, z: i32) -> Self {
        Self {
            section: ChunkSections::new(24, -64),
            heightmap: std::sync::Mutex::new(ChunkHeightmaps::default()),
            x,
            z,
            block_ticks: ChunkTickScheduler::default(),
            fluid_ticks: ChunkTickScheduler::default(),
            pending_block_entities: std::sync::Mutex::new(FxHashMap::default()),
            light_engine: std::sync::Mutex::new(ChunkLight::default()),
            light_populated: std::sync::atomic::AtomicBool::new(false),
            lighting_invalid: AtomicBool::new(false),
            status: ChunkStatus::Full,
            blending_data: None,
            dirty: io::DirtyFlag::new(false),
            inhabited_time: std::sync::atomic::AtomicU64::new(0),
            custom_data: std::sync::Mutex::new(NbtCompound::new()),
        }
    }

    #[must_use]
    pub fn empty_sync(x: i32, z: i32) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::empty(x, z))
    }

    /// Returns the replaced block state ID
    pub fn set_block_absolute_y(
        &self,
        relative_x: usize,
        y: i32,
        relative_z: usize,
        block_state_id: BlockStateId,
    ) -> BlockStateId {
        self.set_block_absolute_y_if(relative_x, y, relative_z, block_state_id, |_| true)
            .unwrap_or(Block::AIR.default_state.id)
    }

    /// Sets the block only when `condition` accepts the current state, atomically.
    /// Returns the replaced block state ID, or `None` when nothing was written.
    pub fn set_block_absolute_y_if(
        &self,
        relative_x: usize,
        y: i32,
        relative_z: usize,
        block_state_id: BlockStateId,
        condition: impl FnOnce(BlockStateId) -> bool,
    ) -> Option<BlockStateId> {
        let min_y = self.section.min_y;
        let y_rel = y - min_y;
        if y_rel < 0 {
            return None;
        }
        let relative_y = y_rel as usize;

        let old = self.section.set_block_if_no_heightmap_update(
            relative_x,
            relative_y,
            relative_z,
            block_state_id,
            condition,
        )?;
        if old != block_state_id {
            let state = BlockState::from_id(block_state_id);
            self.update_heightmap(relative_x, relative_y, relative_z, state);
        }
        Some(old)
    }

    /// Sets multiple blocks in the chunk at absolute Y coordinates in a single batch.
    ///
    /// This acquires section write locks and heightmap locks once across all updates,
    /// significantly speeding up bulk block modifications.
    ///
    /// Returns a list of `(relative_x, y, relative_z, replaced_block_state_id)`.
    pub fn set_blocks_batch(
        &self,
        updates: impl IntoIterator<Item = (usize, i32, usize, BlockStateId)>,
    ) -> Vec<(usize, i32, usize, BlockStateId)> {
        let min_y = self.section.min_y;
        let mut results = Vec::new();

        let mut sections = self
            .section
            .block_sections
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut random_tick_sections_guard = self
            .section
            .random_tick_sections
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut changed_columns = FxHashSet::default();
        let mut modified = false;

        for (rel_x, y, rel_z, new_state_id) in updates {
            let y_rel = y - min_y;
            if y_rel < 0 {
                continue;
            }
            let rel_y = y_rel as usize;
            let section_index = rel_y / BlockPalette::SIZE;
            let sub_y = rel_y % BlockPalette::SIZE;

            if let Some(section) = sections.get_mut(section_index) {
                let replaced_id = section.set(rel_x, sub_y, rel_z, new_state_id);
                if replaced_id != new_state_id {
                    modified = true;
                    changed_columns.insert((rel_x, rel_z));
                    if (has_random_ticks(new_state_id) || has_random_ticking_fluid(new_state_id))
                        && random_tick_sections_guard.is_none()
                    {
                        let new_cache = vec![RandomTickSectionCache::default(); self.section.count]
                            .into_boxed_slice();
                        *random_tick_sections_guard = Some(new_cache);
                    }

                    if let Some(random_tick_sections) = random_tick_sections_guard.as_mut() {
                        let random_tick_cache = &mut random_tick_sections[section_index];
                        if has_random_ticks(replaced_id) {
                            random_tick_cache.random_ticking_block_count = random_tick_cache
                                .random_ticking_block_count
                                .saturating_sub(1);
                        }
                        if has_random_ticking_fluid(replaced_id) {
                            random_tick_cache.random_ticking_fluid_count = random_tick_cache
                                .random_ticking_fluid_count
                                .saturating_sub(1);
                        }
                        if has_random_ticks(new_state_id) {
                            random_tick_cache.random_ticking_block_count = random_tick_cache
                                .random_ticking_block_count
                                .saturating_add(1);
                        }
                        if has_random_ticking_fluid(new_state_id) {
                            random_tick_cache.random_ticking_fluid_count = random_tick_cache
                                .random_ticking_fluid_count
                                .saturating_add(1);
                        }

                        self.section
                            .randomly_ticking_mask
                            .set(section_index, random_tick_cache.is_randomly_ticking());
                    }
                }
                results.push((rel_x, y, rel_z, replaced_id));
            }
        }

        if modified {
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        drop(sections);
        drop(random_tick_sections_guard);

        if !changed_columns.is_empty() {
            let mut heightmap = self
                .heightmap
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let highest_non_empty_subchunk = self.get_highest_non_empty_subchunk();
            for (x, z) in changed_columns {
                self.populate_heightmaps(&mut heightmap, highest_non_empty_subchunk, x, z);
            }
        }

        results
    }

    fn update_heightmap(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        block_state: &BlockState,
    ) {
        let mut heightmap = self
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let min_y = self.section.min_y;
        let x = relative_x as i32;
        let y = relative_y as i32 + min_y;
        let z = relative_z as i32;

        for &hm_type in &[
            ChunkHeightmapType::WorldSurface,
            ChunkHeightmapType::MotionBlocking,
            ChunkHeightmapType::MotionBlockingNoLeaves,
        ] {
            // Heightmap.update takes local X, absolute Y, then local Z.
            heightmap.update(hm_type, x, y, z, block_state, min_y, |y_at| {
                let id = self
                    .section
                    .get_block_absolute_y(relative_x, y_at, relative_z)
                    .unwrap_or(BlockStateId::AIR);
                BlockState::from_id(id)
            });
        }
    }

    /// Gets the given block in the chunk
    #[inline]
    #[must_use]
    pub fn get_relative_block(
        &self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
    ) -> Option<BlockStateId> {
        self.section
            .get_relative_block(relative_x, relative_y, relative_z)
    }

    /// Sets the given block in the chunk
    #[inline]
    pub fn set_relative_block(
        &mut self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        block_state_id: BlockStateId,
    ) {
        let state = BlockState::from_id(block_state_id);
        self.update_heightmap(relative_x, relative_y, relative_z, state);
        self.section
            .set_relative_block(relative_x, relative_y, relative_z, block_state_id);
    }

    /// Sets the given block in the chunk, returning the old block
    /// Contrary to `set_block` this does not update the heightmap.
    ///
    /// Only use this if you know you don't need to update the heightmap
    /// or if you manually set the heightmap in `empty_with_heightmap`
    #[inline]
    pub fn set_block_no_heightmap_update(
        &mut self,
        relative_x: usize,
        relative_y: usize,
        relative_z: usize,
        block_state_id: BlockStateId,
    ) {
        self.section
            .set_relative_block(relative_x, relative_y, relative_z, block_state_id);
    }

    //TODO: Tracking heightmaps update.
    pub fn calculate_heightmap(&self) -> ChunkHeightmaps {
        let highest_non_empty_subchunk = self.get_highest_non_empty_subchunk();
        let mut heightmaps = ChunkHeightmaps::new(self.section.count as i32 * 16);

        for x in 0..16 {
            for z in 0..16 {
                self.populate_heightmaps(&mut heightmaps, highest_non_empty_subchunk, x, z);
            }
        }

        // log::info!("WorldSurface:");
        // heightmaps.log_heightmap(ChunkHeightmapType::WorldSurface, self.section.min_y);
        // log::info!("MotionBlocking:");
        // heightmaps.log_heightmap(ChunkHeightmapType::MotionBlocking, self.section.min_y);
        // log::info!("min_y: {}", self.section.min_y);
        heightmaps
    }

    #[inline]
    fn populate_heightmaps(
        &self,
        heightmaps: &mut ChunkHeightmaps,
        start_sub_chunk: usize,
        x: usize,
        z: usize,
    ) {
        // Heightmap.primeHeightmaps scans absolute section heights.
        let start_height = (start_sub_chunk as i32) * 16 + self.section.min_y + 15;
        let mut has_found = [false, false, false];

        for y in (self.section.min_y..=start_height).rev() {
            let Some(state_id) = self.section.get_block_absolute_y(x, y, z) else {
                continue;
            };
            let block_state = BlockState::from_id(state_id);

            for hm_type in [
                ChunkHeightmapType::WorldSurface,
                ChunkHeightmapType::MotionBlocking,
                ChunkHeightmapType::MotionBlockingNoLeaves,
            ] {
                let idx = hm_type as usize;
                if !has_found[idx] && hm_type.is_opaque(block_state) {
                    heightmaps.set(hm_type, x as i32, z as i32, y, self.section.min_y);
                    has_found[idx] = true;
                }
            }

            if has_found.iter().all(|&found| found) {
                return;
            }
        }

        for (idx, is_set) in has_found.iter().enumerate() {
            if !(*is_set) && let Ok(hm_type) = idx.try_into() {
                heightmaps.set(
                    hm_type,
                    x as i32,
                    z as i32,
                    self.section.min_y - 1,
                    self.section.min_y,
                );
            }
        }
    }

    #[must_use]
    pub fn get_highest_non_empty_subchunk(&self) -> usize {
        self.section
            .block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .enumerate()
            .rev()
            .find(|(_, sub)| !sub.has_only_air())
            .map_or(0, |(idx, _)| idx)
    }
}

#[derive(Error, Debug)]
pub enum ChunkParsingError {
    #[error("Failed reading chunk status {0}")]
    FailedReadStatus(pumpkin_nbt::Error),
    #[error("The chunk isn't generated yet")]
    ChunkNotGenerated,
    #[error("Error deserializing chunk: {0}")]
    ErrorDeserializingChunk(String),
}

#[derive(Error, Debug)]
pub enum ChunkSerializingError {
    #[error("Error serializing chunk: {0}")]
    ErrorSerializingChunk(pumpkin_nbt::Error),
}

#[cfg(test)]
mod tests {
    use super::{ChunkData, ChunkHeightmapType};
    #[test]
    fn incremental_heightmap_uses_absolute_y_and_preserves_the_column() {
        let chunk = ChunkData::empty(0, 0);
        chunk.set_block_absolute_y(10, 64, 2, Block::STONE.default_state.id);
        chunk.set_block_absolute_y(10, 66, 2, Block::STONE.default_state.id);
        let maps = chunk
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(
            maps.get(
                ChunkHeightmapType::MotionBlocking,
                10,
                2,
                chunk.section.min_y
            ),
            66
        );
        assert_eq!(
            maps.get(
                ChunkHeightmapType::MotionBlocking,
                10,
                3,
                chunk.section.min_y
            ),
            chunk.section.min_y - 1
        );
        drop(maps);
        chunk.set_block_absolute_y(10, 66, 2, Block::AIR.default_state.id);
        let maps = chunk
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(
            maps.get(
                ChunkHeightmapType::MotionBlocking,
                10,
                2,
                chunk.section.min_y
            ),
            64
        );
    }

    #[test]
    fn heightmap_layout_and_priming_follow_dimension_height() {
        let mut chunk = ChunkData::empty(0, 0);
        chunk.section = ChunkSections::new(8, 0);
        chunk
            .section
            .set_block_no_heightmap_update(15, 127, 15, Block::STONE.default_state.id);
        chunk.prime_heightmaps(128);
        let maps = chunk
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(maps.world_surface.as_ref().map(|data| data.len()), Some(32));
        assert_eq!(maps.get(ChunkHeightmapType::WorldSurface, 15, 15, 0), 127);
        assert_eq!(maps.get(ChunkHeightmapType::MotionBlocking, 0, 0, 0), -1);
    }

    use super::{ChunkSections, RandomTickMembership};
    use crate::chunk::palette::BlockPalette;
    use pumpkin_data::{Block, block_properties::has_random_ticks};

    #[test]
    fn random_ticks_above_section_31_do_not_overflow() {
        let mut palettes = vec![BlockPalette::default(); 40];
        palettes[35].set(0, 0, 0, Block::LAVA.default_state.id);
        let (_, membership) = ChunkSections::build_random_tick_sections_cache(&palettes);
        let membership = RandomTickMembership::new(membership);
        assert!(membership.contains(35));
        assert!(!membership.contains(3));
        membership.set(35, false);
        assert!(!membership.any());
    }

    #[test]
    fn random_tick_cache_initializes_from_palette_contents() {
        let mut sections = vec![BlockPalette::default(), BlockPalette::default()];
        sections[1].set(0, 0, 0, Block::LAVA.default_state.id);

        let (cache, _mask) = ChunkSections::build_random_tick_sections_cache(&sections);
        let cache = cache.unwrap();
        assert!(!cache[0].is_randomly_ticking());
        assert!(cache[1].random_ticking_fluid_count > 0);
        assert!(cache[1].is_randomly_ticking());
    }

    #[test]
    fn random_tick_cache_updates_on_block_mutation() {
        let min_y = -64;
        let sections = ChunkSections::new(1, min_y);

        assert!(
            sections
                .random_tick_sections
                .read()
                .unwrap()
                .as_ref()
                .is_none_or(|c| !c[0].is_randomly_ticking()),
            "fresh sections should not be randomly ticking"
        );

        let random_block_state = Block::WHEAT.default_state.id;
        assert!(
            has_random_ticks(random_block_state),
            "test requires a known randomly ticking block state"
        );

        sections.set_block_absolute_y(0, min_y, 0, random_block_state);
        {
            let cache = sections.random_tick_sections.read().unwrap();
            let cache = cache.as_ref().unwrap();
            assert_eq!(cache[0].random_ticking_block_count, 1);
            assert_eq!(cache[0].random_ticking_fluid_count, 0);
            assert!(cache[0].is_randomly_ticking());
        };

        sections.set_block_absolute_y(0, min_y, 0, Block::STONE.default_state.id);
        {
            let cache = sections.random_tick_sections.read().unwrap();
            let cache = cache.as_ref().unwrap();
            assert_eq!(cache[0].random_ticking_block_count, 0);
            assert_eq!(cache[0].random_ticking_fluid_count, 0);
            assert!(!cache[0].is_randomly_ticking());
        };

        sections.set_block_absolute_y(0, min_y, 0, Block::LAVA.default_state.id);
        {
            let cache = sections.random_tick_sections.read().unwrap();
            let cache = cache.as_ref().unwrap();
            assert!(cache[0].random_ticking_fluid_count > 0);
            assert!(cache[0].is_randomly_ticking());
        }
    }

    #[test]
    fn heightmap_is_opaque() {
        use crate::chunk::ChunkHeightmapType;

        let air = Block::AIR.default_state;
        let stone = Block::STONE.default_state;
        let leaves = Block::OAK_LEAVES.default_state;
        let water = Block::WATER.default_state;

        // WORLD_SURFACE: Everything except air
        assert!(!ChunkHeightmapType::WorldSurface.is_opaque(air));
        assert!(ChunkHeightmapType::WorldSurface.is_opaque(stone));
        assert!(ChunkHeightmapType::WorldSurface.is_opaque(leaves));
        assert!(ChunkHeightmapType::WorldSurface.is_opaque(water));

        // MOTION_BLOCKING: Blocks movement OR is liquid
        assert!(!ChunkHeightmapType::MotionBlocking.is_opaque(air));
        assert!(ChunkHeightmapType::MotionBlocking.is_opaque(stone));
        assert!(ChunkHeightmapType::MotionBlocking.is_opaque(leaves)); // Leaves block movement
        assert!(ChunkHeightmapType::MotionBlocking.is_opaque(water)); // Water is liquid

        // MOTION_BLOCKING_NO_LEAVES: Blocks movement OR is liquid, but NOT leaves
        assert!(!ChunkHeightmapType::MotionBlockingNoLeaves.is_opaque(air));
        assert!(ChunkHeightmapType::MotionBlockingNoLeaves.is_opaque(stone));
        assert!(!ChunkHeightmapType::MotionBlockingNoLeaves.is_opaque(leaves)); // Excludes leaves
        assert!(ChunkHeightmapType::MotionBlockingNoLeaves.is_opaque(water)); // Water is liquid
    }

    #[test]
    fn chunk_custom_data() {
        use pumpkin_nbt::tag::NbtTag;

        let chunk = super::ChunkData::empty(0, 0);
        assert!(!chunk.has_custom_data("my_plugin", "test_key"));
        assert_eq!(chunk.get_custom_data("my_plugin", "test_key"), None);

        chunk.set_custom_data(
            "my_plugin",
            "test_key",
            NbtTag::String("hello_pumpkin".into()),
        );
        assert!(chunk.has_custom_data("my_plugin", "test_key"));
        assert_eq!(
            chunk.get_custom_data("my_plugin", "test_key"),
            Some(NbtTag::String("hello_pumpkin".into()))
        );

        chunk.set_custom_data("my_plugin", "number_key", NbtTag::Int(42));
        assert_eq!(
            chunk.get_custom_data("my_plugin", "number_key"),
            Some(NbtTag::Int(42))
        );

        chunk.remove_custom_data("my_plugin", "test_key");
        assert!(!chunk.has_custom_data("my_plugin", "test_key"));
        assert!(chunk.has_custom_data("my_plugin", "number_key"));
    }
}
