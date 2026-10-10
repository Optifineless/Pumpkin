use std::sync::Arc;
use std::{
    path::PathBuf,
    sync::{
        RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use bytes::Bytes;
use pumpkin_data::{Block, BlockStateId, chunk::ChunkStatus, fluid::Fluid};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::resource_location::FromResourceLocation;
use rustc_hash::FxHashMap;

use crate::{
    block::{block_state_from_nbt, block_state_to_nbt},
    chunk::{
        ChunkEntityData, ChunkReadingError, ChunkSerializingError,
        format::anvil::{SingleChunkDataSerializer, WORLD_DATA_VERSION},
        io::{Dirtiable, file_manager::PathFromLevelFolder},
    },
    generation::section_coords,
    level::LevelFolder,
    tick::{ScheduledTick, scheduler::ChunkTickScheduler},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;

use super::{
    ChunkData, ChunkHeightmaps, ChunkLight, ChunkParsingError, ChunkSections,
    palette::{BiomePalette, BlockPalette},
};
pub mod anvil;
pub mod linear;
pub mod pump;
mod section_bounds;

impl SingleChunkDataSerializer for ChunkData {
    #[inline]
    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        Self::internal_from_bytes(bytes, pos, &pumpkin_data::dimension::Dimension::OVERWORLD)
            .map_err(ChunkReadingError::ParsingError)
    }

    fn from_bytes_in_dimension(
        bytes: &Bytes,
        pos: Vector2<i32>,
        dimension: &pumpkin_data::dimension::Dimension,
    ) -> Result<Self, ChunkReadingError> {
        Self::internal_from_bytes(bytes, pos, dimension).map_err(ChunkReadingError::ParsingError)
    }

    #[inline]
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        Ok(self.internal_to_bytes())
    }

    #[inline]
    fn position(&self) -> (i32, i32) {
        (self.x, self.z)
    }
}

impl PathFromLevelFolder for ChunkData {
    #[inline]
    fn file_path(folder: &LevelFolder, file_name: &str) -> PathBuf {
        folder.region_folder.join(file_name)
    }
}

impl Dirtiable for ChunkData {
    fn dirty_version(&self) -> Option<u64> {
        Some(self.dirty.version())
    }
    fn clear_published(&self, version: u64) {
        self.dirty.clear_published(version);
    }

    #[inline]
    fn mark_dirty(&self, flag: bool) {
        self.dirty.store(flag, Ordering::Relaxed);
    }

    #[inline]
    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
}

/// The section stores `Y` as a byte, short, int or long depending on who wrote
/// the file. The datafixer writes ints. Reading only bytes would map every int
/// section to `Y = 0`, and they would overwrite each other.
fn section_y(section: &NbtCompound) -> Result<i32, ChunkParsingError> {
    use pumpkin_nbt::tag::NbtTag;
    let value = match section.get("Y") {
        Some(NbtTag::Byte(value)) => Some(i32::from(*value)),
        Some(NbtTag::Short(value)) => Some(i32::from(*value)),
        Some(NbtTag::Int(value)) => Some(*value),
        Some(NbtTag::Long(value)) => i32::try_from(*value).ok(),
        _ => None,
    };
    value.ok_or_else(|| ChunkParsingError::ErrorDeserializingChunk("Invalid section Y".into()))
}

fn extract_u16_array(tag: &pumpkin_nbt::tag::NbtTag) -> Option<Box<[BlockStateId]>> {
    match tag {
        pumpkin_nbt::tag::NbtTag::IntArray(arr) => Some(
            arr.iter()
                .map(|&x| BlockStateId::new_or_air(x as u16))
                .collect(),
        ),
        pumpkin_nbt::tag::NbtTag::ByteArray(arr) => Some(
            arr.iter()
                .map(|&x| BlockStateId::new_or_air(x as u16))
                .collect(),
        ),
        pumpkin_nbt::tag::NbtTag::LongArray(arr) => Some(
            arr.iter()
                .map(|&x| BlockStateId::new_or_air(x as u16))
                .collect(),
        ),
        pumpkin_nbt::tag::NbtTag::List(list) => {
            let ids: Box<[BlockStateId]> = list
                .iter()
                .map(|t| match t {
                    pumpkin_nbt::tag::NbtTag::Int(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Short(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Byte(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Long(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Compound(compound) => block_state_from_nbt(compound),
                    _ => BlockStateId::AIR,
                })
                .collect();
            Some(ids)
        }
        _ => None,
    }
}

fn extract_u8_array(tag: &pumpkin_nbt::tag::NbtTag) -> Option<Box<[u8]>> {
    match tag {
        pumpkin_nbt::tag::NbtTag::ByteArray(arr) => Some(arr.iter().map(|&x| x as u8).collect()),
        pumpkin_nbt::tag::NbtTag::IntArray(arr) => Some(arr.iter().map(|&x| x as u8).collect()),
        pumpkin_nbt::tag::NbtTag::List(list) => {
            let bytes: Box<[u8]> = list
                .iter()
                .map(|t| match t {
                    pumpkin_nbt::tag::NbtTag::Byte(x) => *x as u8,
                    pumpkin_nbt::tag::NbtTag::Int(x) => *x as u8,
                    pumpkin_nbt::tag::NbtTag::Short(x) => *x as u8,
                    pumpkin_nbt::tag::NbtTag::String(s) => {
                        let name = s.strip_prefix("minecraft:").unwrap_or(s);
                        pumpkin_data::biome::Biome::from_name(name).map_or(0, |b| b.id)
                    }
                    _ => 0,
                })
                .collect();
            Some(bytes)
        }
        _ => None,
    }
}

fn parse_scheduled_tick<T>(nbt: &pumpkin_nbt::compound::NbtCompound) -> Option<ScheduledTick<T>>
where
    T: FromResourceLocation,
{
    // SavedTick.codec/unpack preserve long delays and schedule overdue ticks on the next tick.
    ScheduledTick::from_nbt_compound(nbt)
}

impl ChunkData {
    #[allow(clippy::too_many_lines)]
    pub fn internal_from_bytes(
        chunk_data: &[u8],
        position: Vector2<i32>,
        dimension: &pumpkin_data::dimension::Dimension,
    ) -> Result<Self, ChunkParsingError> {
        let is_named = chunk_data.len() >= 3
            && chunk_data[0] == 0x0a
            && chunk_data[1] == 0x00
            && chunk_data[2] == 0x00;

        let mut cursor = std::io::Cursor::new(chunk_data);
        let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
        let nbt = if is_named {
            pumpkin_nbt::Nbt::read(&mut reader)
        } else {
            pumpkin_nbt::Nbt::read_unnamed(&mut reader)
        }
        .map_err(|e| ChunkParsingError::ErrorDeserializingChunk(e.to_string()))?;

        let root_tag = nbt.root_tag;

        let x_pos = root_tag.get_int("xPos").ok_or_else(|| {
            ChunkParsingError::ErrorDeserializingChunk("Missing xPos".to_string())
        })?;
        let z_pos = root_tag.get_int("zPos").ok_or_else(|| {
            ChunkParsingError::ErrorDeserializingChunk("Missing zPos".to_string())
        })?;

        if x_pos != position.x || z_pos != position.y {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Expected data for chunk {},{} but got it for {},{}!",
                position.x, position.y, x_pos, z_pos,
            )));
        }

        // SerializableChunkData.parse/read allocates from LevelHeightAccessor, never disk Y values.
        let bounds = section_bounds::SectionBounds::new(&root_tag, dimension)?;
        let min_y_section = bounds.min;
        let section_count = bounds.count;
        let mut block_lights = vec![LightContainer::Empty(0); section_count];
        let mut sky_lights = vec![LightContainer::Empty(0); section_count];
        let mut block_palettes = vec![BlockPalette::default(); section_count];
        let mut biome_palettes = vec![BiomePalette::default(); section_count];
        let mut lighting_invalid = false;
        if let Some(sections_list) = root_tag.get_list("sections") {
            for section_tag in sections_list {
                if let pumpkin_nbt::tag::NbtTag::Compound(section_compound) = section_tag {
                    let y = section_y(section_compound)?;
                    let Some(index) = bounds.index(y, section_compound)? else {
                        continue;
                    };

                    let block_light = section_compound
                        .get("BlockLight")
                        .and_then(|tag| tag.extract_byte_array())
                        .map(|arr| -> Box<[u8]> {
                            // SAFETY: `arr` is an `i8` slice (`&[i8]`). `u8` and `i8` have identical memory layout, alignment (1 byte), and lifetime.
                            unsafe {
                                Box::from(std::slice::from_raw_parts(
                                    arr.as_ptr().cast::<u8>(),
                                    arr.len(),
                                ))
                            }
                        });

                    let sky_light = section_compound
                        .get("SkyLight")
                        .and_then(|tag| tag.extract_byte_array())
                        .map(|arr| -> Box<[u8]> {
                            // SAFETY: `arr` is an `i8` slice (`&[i8]`). `u8` and `i8` have identical memory layout, alignment (1 byte), and lifetime.
                            unsafe {
                                Box::from(std::slice::from_raw_parts(
                                    arr.as_ptr().cast::<u8>(),
                                    arr.len(),
                                ))
                            }
                        });

                    // `Full` skips the length check `LightContainer::new` makes,
                    // and every reader indexes it up to `ARRAY_SIZE`, so a
                    // short array from disk is dropped instead of stored.
                    // discarded layers must re-enter normal lighting (SerializableChunkData.read).
                    lighting_invalid |= ["BlockLight", "SkyLight"].iter().any(|name| {
                        section_compound.get(name).is_some_and(|tag| {
                            tag.extract_byte_array()
                                .is_none_or(|data| data.len() != LightContainer::ARRAY_SIZE)
                        })
                    });
                    block_lights[index] = block_light
                        .filter(|data| data.len() == LightContainer::ARRAY_SIZE)
                        .map_or(LightContainer::Empty(0), LightContainer::Full);
                    sky_lights[index] = sky_light
                        .filter(|data| data.len() == LightContainer::ARRAY_SIZE)
                        .map_or(LightContainer::Empty(0), LightContainer::Full);

                    if let Some(bs_compound) = section_compound.get_compound("block_states") {
                        let data = bs_compound
                            .get_long_array("data")
                            .map(|arr| arr.to_vec().into_boxed_slice());
                        let palette = bs_compound
                            .get("palette")
                            .and_then(extract_u16_array)
                            .unwrap_or_else(|| vec![BlockStateId::AIR].into_boxed_slice());

                        block_palettes[index] =
                            BlockPalette::from_disk_nbt(ChunkSectionBlockStates { data, palette });
                    } else {
                        block_palettes[index] = BlockPalette::default();
                    }

                    if let Some(b_compound) = section_compound.get_compound("biomes") {
                        let data = b_compound
                            .get_long_array("data")
                            .map(|arr| arr.to_vec().into_boxed_slice());
                        let palette = b_compound
                            .get("palette")
                            .and_then(extract_u8_array)
                            .unwrap_or_else(|| vec![0].into_boxed_slice());

                        biome_palettes[index] =
                            BiomePalette::from_disk_nbt(ChunkSectionBiomes { data, palette });
                    } else {
                        biome_palettes[index] = BiomePalette::default();
                    }
                }
            }
        }

        // Assemble the LightEngine
        let light_engine = ChunkLight {
            block_light: block_lights.into_boxed_slice(),
            sky_light: sky_lights.into_boxed_slice(),
        };

        // Assemble the ChunkSections
        let min_y = section_coords::section_to_block(min_y_section);
        let (random_tick_sections, randomly_ticking_mask) =
            ChunkSections::build_random_tick_sections_cache(&block_palettes);
        let section = ChunkSections {
            count: block_palettes.len(),
            block_sections: RwLock::new(block_palettes.into_boxed_slice()),
            random_tick_sections: RwLock::new(random_tick_sections),
            randomly_ticking_mask: super::RandomTickMembership::new(randomly_ticking_mask),
            biome_sections: RwLock::new(biome_palettes.into_boxed_slice()),
            min_y,
        };

        let heightmaps = root_tag.get_compound("Heightmaps").map_or(
            ChunkHeightmaps {
                // validate against the configured dimension at the load boundary.
                height_bits: ChunkHeightmaps::new(section_count as i32 * 16).height_bits,
                world_surface: None,
                motion_blocking: None,
                motion_blocking_no_leaves: None,
            },
            // the load boundary validates length against the dimension
            // and primes missing maps before publishing this chunk.
            |h_compound| ChunkHeightmaps {
                height_bits: ChunkHeightmaps::new(section_count as i32 * 16).height_bits,
                world_surface: h_compound
                    .get_long_array("WORLD_SURFACE")
                    .map(|a| a.to_vec().into_boxed_slice()),
                motion_blocking: h_compound
                    .get_long_array("MOTION_BLOCKING")
                    .map(|a| a.to_vec().into_boxed_slice()),
                motion_blocking_no_leaves: h_compound
                    .get_long_array("MOTION_BLOCKING_NO_LEAVES")
                    .map(|a| a.to_vec().into_boxed_slice()),
            },
        );
        let mut block_ticks = Vec::new();
        if let Some(list) = root_tag.get_list("block_ticks") {
            for tag in list {
                if let pumpkin_nbt::tag::NbtTag::Compound(compound) = tag
                    && let Some(tick) = parse_scheduled_tick::<&'static Block>(compound)
                {
                    block_ticks.push(tick);
                }
            }
        }

        let mut fluid_ticks = Vec::new();
        if let Some(list) = root_tag.get_list("fluid_ticks") {
            for tag in list {
                if let pumpkin_nbt::tag::NbtTag::Compound(compound) = tag
                    && let Some(tick) = parse_scheduled_tick::<&'static Fluid>(compound)
                {
                    fluid_ticks.push(tick);
                }
            }
        }

        let mut block_entities = FxHashMap::default();
        if let Some(list) = root_tag.get_list("block_entities") {
            for tag in list {
                if let pumpkin_nbt::tag::NbtTag::Compound(nbt) = tag
                    && let Some(x) = nbt.get_int("x")
                    && let Some(y) = nbt.get_int("y")
                    && let Some(z) = nbt.get_int("z")
                {
                    block_entities.insert(BlockPos::new(x, y, z), nbt.clone());
                }
            }
        }

        let light_correct = root_tag.get_bool("isLightOn").unwrap_or(false);

        let status_str = root_tag.get_string("Status").unwrap_or("minecraft:empty");
        let status = match status_str {
            "minecraft:structure_starts" => ChunkStatus::StructureStarts,
            "minecraft:structure_references" => ChunkStatus::StructureReferences,
            "minecraft:biomes" => ChunkStatus::Biomes,
            "minecraft:terrain" | "minecraft:noise" | "minecraft:surface" | "minecraft:carvers" => {
                ChunkStatus::Terrain
            }
            "minecraft:features" => ChunkStatus::Features,
            "minecraft:initialize_light" => ChunkStatus::InitializeLight,
            "minecraft:light" => ChunkStatus::Light,
            "minecraft:spawn" => ChunkStatus::Spawn,
            "minecraft:full" => ChunkStatus::Full,
            _ => ChunkStatus::Empty,
        };

        let custom_data = root_tag
            .get_compound("PumpkinCustomData")
            .or_else(|| root_tag.get_compound("BukkitValues"))
            .cloned()
            .unwrap_or_default();

        Ok(Self {
            section,
            heightmap: std::sync::Mutex::new(heightmaps),
            x: position.x,
            z: position.y,
            // This chunk is read from disk, so it has not been modified
            dirty: crate::chunk::io::DirtyFlag::new(false),
            block_ticks: ChunkTickScheduler::from_iter(block_ticks),
            fluid_ticks: ChunkTickScheduler::from_iter(fluid_ticks),
            pending_block_entities: std::sync::Mutex::new(block_entities),
            light_engine: std::sync::Mutex::new(light_engine),
            light_populated: AtomicBool::new(light_correct && !lighting_invalid),
            lighting_invalid: AtomicBool::new(lighting_invalid),
            status,
            blending_data: None,
            inhabited_time: AtomicU64::new(root_tag.get_long("InhabitedTime").unwrap_or(0) as u64),
            custom_data: std::sync::Mutex::new(custom_data),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn internal_to_bytes(&self) -> Bytes {
        use pumpkin_nbt::tag::NbtTag;

        fn extract_light_ref(light: Option<&LightContainer>) -> Option<&[u8]> {
            match light {
                Some(LightContainer::Full(data)) => Some(data.as_ref()),
                _ => None,
            }
        }

        let is_light_correct = self
            .light_populated
            .load(std::sync::atomic::Ordering::Relaxed);

        let block_entities_nbt = {
            let entities_guard = self
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entities_guard.values().cloned().collect::<Vec<_>>()
        };

        let light_lock = self
            .light_engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let heightmap_lock = self
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let block_lock = self
            .section
            .block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let biome_lock = self
            .section
            .biome_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let min_section_y = (self.section.min_y >> 4) as i8;

        let mut root_compound = NbtCompound::new();
        root_compound.put_int("DataVersion", WORLD_DATA_VERSION);
        root_compound.put_int("xPos", self.x);
        root_compound.put_int("zPos", self.z);
        root_compound.put_int("yPos", section_coords::block_to_section(self.section.min_y));

        let status_str = match self.status {
            ChunkStatus::Empty => "minecraft:empty",
            ChunkStatus::StructureStarts => "minecraft:structure_starts",
            ChunkStatus::StructureReferences => "minecraft:structure_references",
            ChunkStatus::Biomes => "minecraft:biomes",
            ChunkStatus::Terrain => "minecraft:terrain",
            ChunkStatus::Features => "minecraft:features",
            ChunkStatus::InitializeLight => "minecraft:initialize_light",
            ChunkStatus::Light => "minecraft:light",
            ChunkStatus::Spawn => "minecraft:spawn",
            ChunkStatus::Full => "minecraft:full",
        };
        root_compound.put_string("Status", status_str.to_string());

        let mut heightmaps_compound = NbtCompound::new();
        if let Some(ref arr) = heightmap_lock.world_surface {
            heightmaps_compound.put("WORLD_SURFACE", NbtTag::LongArray(arr.to_vec()));
        }
        if let Some(ref arr) = heightmap_lock.motion_blocking {
            heightmaps_compound.put("MOTION_BLOCKING", NbtTag::LongArray(arr.to_vec()));
        }
        if let Some(ref arr) = heightmap_lock.motion_blocking_no_leaves {
            heightmaps_compound.put("MOTION_BLOCKING_NO_LEAVES", NbtTag::LongArray(arr.to_vec()));
        }
        root_compound.put_compound("Heightmaps", heightmaps_compound);

        let mut sections_list = Vec::new();
        for i in 0..self.section.count {
            let mut section_comp = NbtCompound::new();
            let y_val = i as i8 + min_section_y;
            section_comp.put_byte("Y", y_val);

            // block_states
            let block_states_nbt = block_lock[i].to_disk_nbt();
            let mut bs_comp = NbtCompound::new();
            if let Some(ref data_arr) = block_states_nbt.data {
                bs_comp.put("data", NbtTag::LongArray(data_arr.to_vec()));
            }
            let palette_tags: Vec<NbtTag> = block_states_nbt
                .palette
                .iter()
                .map(|&id| NbtTag::Compound(block_state_to_nbt(id)))
                .collect();
            bs_comp.put_list("palette", palette_tags);
            section_comp.put_compound("block_states", bs_comp);

            // biomes
            let biomes_nbt = biome_lock[i].to_disk_nbt();
            let mut b_comp = NbtCompound::new();
            if let Some(ref data_arr) = biomes_nbt.data {
                b_comp.put("data", NbtTag::LongArray(data_arr.to_vec()));
            }
            let biome_palette_tags: Vec<NbtTag> = biomes_nbt
                .palette
                .iter()
                .map(|&val| {
                    let name = pumpkin_data::biome::Biome::from_id(val)
                        .map_or("plains", |b| b.registry_id);
                    let full_name = if name.starts_with("minecraft:") {
                        name.to_string()
                    } else {
                        format!("minecraft:{name}")
                    };
                    NbtTag::String(full_name.into())
                })
                .collect();
            b_comp.put_list("palette", biome_palette_tags);
            section_comp.put_compound("biomes", b_comp);

            // block_light
            if let Some(light_data) = extract_light_ref(light_lock.block_light.get(i)) {
                let bytes: Box<[i8]> = light_data.iter().map(|&x| x as i8).collect();
                section_comp.put("BlockLight", NbtTag::ByteArray(bytes));
            }

            // sky_light
            if let Some(light_data) = extract_light_ref(light_lock.sky_light.get(i)) {
                let bytes: Box<[i8]> = light_data.iter().map(|&x| x as i8).collect();
                section_comp.put("SkyLight", NbtTag::ByteArray(bytes));
            }

            sections_list.push(NbtTag::Compound(section_comp));
        }
        root_compound.put_list("sections", sections_list);

        let mut block_ticks_list = Vec::new();
        for tick in self.block_ticks.to_vec() {
            block_ticks_list.push(NbtTag::Compound(tick.to_nbt_compound()));
        }
        root_compound.put_list("block_ticks", block_ticks_list);

        let mut fluid_ticks_list = Vec::new();
        for tick in self.fluid_ticks.to_vec() {
            fluid_ticks_list.push(NbtTag::Compound(tick.to_nbt_compound()));
        }
        root_compound.put_list("fluid_ticks", fluid_ticks_list);

        let mut block_entities_list = Vec::new();
        for entity_comp in block_entities_nbt {
            block_entities_list.push(NbtTag::Compound(entity_comp));
        }
        root_compound.put_list("block_entities", block_entities_list);

        root_compound.put_bool("isLightOn", is_light_correct);
        root_compound.put_long(
            "InhabitedTime",
            self.inhabited_time.load(Ordering::Relaxed) as i64,
        );

        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !custom_data.is_empty() {
            root_compound.put_compound("PumpkinCustomData", custom_data.clone());
        }

        let nbt = pumpkin_nbt::Nbt::from(root_compound);
        nbt.write()
    }

    pub(crate) fn set_custom_data(
        &self,
        namespace: &str,
        key: &str,
        value: pumpkin_nbt::tag::NbtTag,
    ) {
        let mut custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut namespace_data = custom_data
            .child_tags
            .remove(namespace)
            .and_then(|tag| match tag {
                pumpkin_nbt::tag::NbtTag::Compound(compound) => Some(compound),
                _ => None,
            })
            .unwrap_or_default();

        namespace_data.child_tags.insert(key.into(), value);
        custom_data.child_tags.insert(
            namespace.into(),
            pumpkin_nbt::tag::NbtTag::Compound(namespace_data),
        );
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn get_custom_data(&self, namespace: &str, key: &str) -> Option<pumpkin_nbt::tag::NbtTag> {
        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        custom_data
            .get(namespace)?
            .extract_compound()?
            .get(key)
            .cloned()
    }

    pub(crate) fn remove_custom_data(&self, namespace: &str, key: &str) {
        let mut custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(pumpkin_nbt::tag::NbtTag::Compound(mut namespace_data)) =
            custom_data.child_tags.remove(namespace)
        else {
            return;
        };

        namespace_data.child_tags.remove(key);
        if !namespace_data.is_empty() {
            custom_data.child_tags.insert(
                namespace.into(),
                pumpkin_nbt::tag::NbtTag::Compound(namespace_data),
            );
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn has_custom_data(&self, namespace: &str, key: &str) -> bool {
        self.get_custom_data(namespace, key).is_some()
    }
}

impl PathFromLevelFolder for ChunkEntityData {
    #[inline]
    fn file_path(folder: &LevelFolder, file_name: &str) -> PathBuf {
        folder.entities_folder.join(file_name)
    }
}

impl Dirtiable for ChunkEntityData {
    fn copy_for_load(self: &Arc<Self>) -> Arc<Self> {
        // IOWorker.PendingStore.copyData: activation must not consume a retained save.
        Arc::new(Self {
            x: self.x,
            z: self.z,
            data: std::sync::Mutex::new(
                self.data
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            dormant_records: std::sync::Mutex::new(None),
            live: std::sync::atomic::AtomicBool::new(false),
            dirty: crate::chunk::io::DirtyFlag::new(false),
        })
    }

    fn dirty_version(&self) -> Option<u64> {
        Some(self.dirty.version())
    }
    fn clear_published(&self, version: u64) {
        self.dirty.clear_published(version);
    }

    #[inline]
    fn mark_dirty(&self, flag: bool) {
        self.dirty.store(flag, Ordering::Relaxed);
    }

    #[inline]
    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
}

impl SingleChunkDataSerializer for ChunkEntityData {
    #[inline]
    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        Self::internal_from_bytes(bytes, pos).map_err(ChunkReadingError::ParsingError)
    }

    #[inline]
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        Ok(self.internal_to_bytes())
    }

    #[inline]
    fn position(&self) -> (i32, i32) {
        (self.x, self.z)
    }
}

impl ChunkEntityData {
    fn internal_from_bytes(
        chunk_data: &[u8],
        position: Vector2<i32>,
    ) -> Result<Self, ChunkParsingError> {
        let is_named = chunk_data.len() >= 3
            && chunk_data[0] == 0x0a
            && chunk_data[1] == 0x00
            && chunk_data[2] == 0x00;
        let mut cursor = std::io::Cursor::new(chunk_data);
        let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            pumpkin_nbt::deserializer::NbtStreamReader(&mut cursor),
        );
        let nbt = if is_named {
            pumpkin_nbt::Nbt::read(&mut reader)
        } else {
            pumpkin_nbt::Nbt::read_unnamed(&mut reader)
        }
        .map_err(|e| ChunkParsingError::ErrorDeserializingChunk(e.to_string()))?;

        let pos_array = match (nbt.get_int("Position-X"), nbt.get_int("Position-Z")) {
            (Some(x), Some(z)) => [x, z],
            _ => {
                if let Some(pumpkin_nbt::tag::NbtTag::IntArray(pos)) = nbt.get("Position") {
                    if pos.len() >= 2 {
                        [pos[0], pos[1]]
                    } else {
                        [0, 0]
                    }
                } else {
                    [0, 0]
                }
            }
        };

        if pos_array[0] != position.x || pos_array[1] != position.y {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Expected data for entity chunk {},{} but got it for {},{}!",
                position.x, position.y, pos_array[0], pos_array[1],
            )));
        }

        let entities = match nbt.get("Entities") {
            Some(pumpkin_nbt::tag::NbtTag::List(list)) => list
                .iter()
                .filter_map(|t| match t {
                    pumpkin_nbt::tag::NbtTag::Compound(c) => Some(c.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        Ok(Self {
            x: position.x,
            z: position.y,
            data: std::sync::Mutex::new(entities),
            dormant_records: std::sync::Mutex::new(None),
            live: AtomicBool::new(false),
            dirty: crate::chunk::io::DirtyFlag::new(false),
        })
    }

    fn internal_to_bytes(&self) -> Bytes {
        let mut root = NbtCompound::new();
        root.put_int("DataVersion", WORLD_DATA_VERSION);
        root.put(
            "Position",
            pumpkin_nbt::tag::NbtTag::IntArray(vec![self.x, self.z]),
        );
        let entities_tag: Vec<pumpkin_nbt::tag::NbtTag> = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|c| pumpkin_nbt::tag::NbtTag::Compound(c.clone()))
            .collect();
        root.put_list("Entities", entities_tag);

        let nbt = pumpkin_nbt::Nbt::from(root);
        nbt.write()
    }
}

#[derive(Clone)]
pub struct ChunkSectionBiomes {
    pub(crate) data: Option<Box<[i64]>>,
    pub(crate) palette: Box<[u8]>,
}

#[derive(Clone)]
pub struct ChunkSectionBlockStates {
    pub(crate) data: Option<Box<[i64]>>,
    pub(crate) palette: Box<[BlockStateId]>,
}

#[derive(Debug, Clone)]
pub enum LightContainer {
    Empty(u8),
    Full(Box<[u8]>),
}

impl LightContainer {
    pub const DIM: usize = 16;
    pub const ARRAY_SIZE: usize = Self::DIM * Self::DIM * Self::DIM / 2;

    #[must_use]
    pub fn new_empty(default: u8) -> Self {
        assert!(default <= 15, "Default value must be between 0 and 15");
        Self::Empty(default)
    }

    #[must_use]
    pub fn new(data: Box<[u8]>) -> Self {
        assert!(
            data.len() == Self::ARRAY_SIZE,
            "Data length must be {}",
            Self::ARRAY_SIZE
        );
        Self::Full(data)
    }

    #[must_use]
    pub fn new_filled(default: u8) -> Self {
        assert!(default <= 15, "Default value must be between 0 and 15");
        let value = default << 4 | default;
        Self::Full([value; Self::ARRAY_SIZE].into())
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty(_))
    }

    #[inline]
    const fn index(x: usize, y: usize, z: usize) -> usize {
        y * 16 * 16 + z * 16 + x
    }

    #[inline]
    #[must_use]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        match self {
            Self::Full(data) => {
                let index = Self::index(x, y, z);
                (data[index >> 1] >> (4 * (index & 1))) & 0x0F
            }
            Self::Empty(default) => *default,
        }
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: u8) {
        match self {
            Self::Full(data) => {
                let index = Self::index(x, y, z);
                let shift = 4 * (index & 1);
                let mask = 0x0F << shift;
                data[index >> 1] = (data[index >> 1] & !mask) | (value << shift);
            }
            Self::Empty(default) => {
                if value != *default {
                    *self = Self::new_filled(*default);
                    self.set(x, y, z, value);
                }
            }
        }
    }

    #[inline]
    pub fn set_column_y_range(
        &mut self,
        x: usize,
        z: usize,
        y_start: usize,
        y_end: usize,
        value: u8,
    ) {
        if y_start >= y_end {
            return;
        }
        match self {
            Self::Full(data) => {
                let shift = 4 * (x & 1);
                let mask = 0x0F << shift;
                let val = (value & 0x0F) << shift;
                let mut byte_idx = (y_start * 256 + z * 16 + x) >> 1;
                for _ in y_start..y_end {
                    data[byte_idx] = (data[byte_idx] & !mask) | val;
                    byte_idx += 128;
                }
            }
            Self::Empty(default) => {
                if value != *default {
                    *self = Self::new_filled(*default);
                    self.set_column_y_range(x, z, y_start, y_end, value);
                }
            }
        }
    }

    #[inline]
    pub fn fill(&mut self, value: u8) {
        *self = Self::new_filled(value);
    }
}

impl Default for LightContainer {
    fn default() -> Self {
        Self::new_empty(15)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn discarded_light_is_marked_even_when_saved_chunk_claims_to_be_lit() {
        let mut section = NbtCompound::new();
        section.put_byte("Y", 0);
        section.put(
            "BlockLight",
            NbtTag::ByteArray(vec![0; 1].into_boxed_slice()),
        );
        // Other valid complex lighting cannot suppress repair of the discarded layer.
        section.put(
            "SkyLight",
            NbtTag::ByteArray(vec![0x12; 2048].into_boxed_slice()),
        );
        let mut nbt = test_chunk(vec![section]);
        nbt.root_tag.put_int(
            "yPos",
            pumpkin_data::dimension::Dimension::OVERWORLD
                .min_y
                .div_euclid(16),
        );
        nbt.root_tag.put_bool("isLightOn", true);
        let bytes = nbt.write();
        let result = ChunkData::internal_from_bytes(
            &bytes,
            pumpkin_util::math::vector2::Vector2::new(0, 0),
            &pumpkin_data::dimension::Dimension::OVERWORLD,
        );
        assert!(result.is_ok());
        if let Ok(chunk) = result {
            assert!(
                chunk
                    .lighting_invalid
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
            assert!(
                !chunk
                    .light_populated
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
        }
    }

    use super::*;
    use pumpkin_data::Block;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_nbt::tag::NbtTag;

    fn test_section(y: i32, block: &str, with_biomes: bool) -> NbtCompound {
        let mut block_states = NbtCompound::new();
        let mut air = NbtCompound::new();
        air.put_string("Name", "minecraft:air".to_string());
        let mut solid = NbtCompound::new();
        solid.put_string("Name", block.to_string());
        block_states.put(
            "palette",
            NbtTag::List(vec![NbtTag::Compound(air), NbtTag::Compound(solid)]),
        );
        block_states.put("data", NbtTag::LongArray(vec![1; 256]));

        let mut section = NbtCompound::new();
        section.put_int("Y", y);
        section.put("block_states", NbtTag::Compound(block_states));
        if with_biomes {
            let mut biomes = NbtCompound::new();
            biomes.put(
                "palette",
                NbtTag::List(vec![NbtTag::String("minecraft:plains".into())]),
            );
            section.put("biomes", NbtTag::Compound(biomes));
        }
        section
    }

    fn test_chunk(sections: Vec<NbtCompound>) -> pumpkin_nbt::Nbt {
        let mut root = NbtCompound::new();
        root.put_int("DataVersion", 4903);
        root.put_int("xPos", 0);
        root.put_int("zPos", 0);
        root.put_string("Status", "minecraft:full".to_string());
        root.put(
            "sections",
            NbtTag::List(sections.into_iter().map(NbtTag::Compound).collect()),
        );
        pumpkin_nbt::Nbt::new(String::new(), root)
    }

    #[test]
    fn saved_tick_delay_preserves_long_delays_without_wrapping() {
        let saved = |t: i32| {
            let mut nbt = NbtCompound::new();
            nbt.put_int("x", 0);
            nbt.put_int("y", 0);
            nbt.put_int("z", 0);
            nbt.put_int("t", t);
            nbt.put_int("p", 0);
            nbt.put_string("i", "minecraft:stone".to_string());
            parse_scheduled_tick::<&'static Block>(&nbt).map(|tick| tick.delay)
        };
        // Vanilla saves a tick that was already due with a delay of 0 or less.
        assert_eq!(saved(-1), Some(0));
        assert_eq!(saved(5000), Some(5000));
    }

    #[test]
    fn chunk_writer_bounds_tick_delays_without_immediate_reload() {
        use crate::tick::{MAX_SAVED_TICK_DELAY, TickPriority};
        let dimension = &pumpkin_data::dimension::Dimension::OVERWORLD;
        let chunk = ChunkData::internal_from_bytes(
            &test_chunk(Vec::new()).write(),
            Vector2::new(0, 0),
            dimension,
        )
        .unwrap();
        for (index, (delay, expected)) in [
            (5000, 5000),
            (MAX_SAVED_TICK_DELAY, i32::MAX),
            (MAX_SAVED_TICK_DELAY + 1, i32::MAX),
            (u32::MAX, i32::MAX),
        ]
        .into_iter()
        .enumerate()
        {
            let pos = BlockPos::new(index as i32, 64, 0);
            let tick = ScheduledTick {
                delay,
                position: pos,
                priority: TickPriority::Normal,
                value: &Block::STONE,
            };
            let saved = tick.to_nbt_compound();
            assert_eq!(saved.get_int("t"), Some(expected));
            chunk.block_ticks.schedule_tick(&tick, index as i64);
            chunk.fluid_ticks.schedule_tick(
                &ScheduledTick {
                    value: &Fluid::WATER,
                    delay,
                    position: pos,
                    priority: TickPriority::Normal,
                },
                index as i64,
            );
        }
        // Exercise the actual chunk writer and reader, not just the standalone tick codec.
        let reloaded = ChunkData::internal_from_bytes(
            &chunk.internal_to_bytes(),
            Vector2::new(0, 0),
            dimension,
        )
        .unwrap();
        let expected = [
            5000,
            MAX_SAVED_TICK_DELAY,
            MAX_SAVED_TICK_DELAY,
            MAX_SAVED_TICK_DELAY,
        ];
        assert_eq!(
            reloaded
                .block_ticks
                .to_vec()
                .iter()
                .map(|tick| tick.delay)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            reloaded
                .fluid_ticks
                .to_vec()
                .iter()
                .map(|tick| tick.delay)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            chunk
                .block_ticks
                .to_vec()
                .iter()
                .map(|tick| tick.delay)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(reloaded.block_ticks.step_tick().is_empty());
        assert!(reloaded.fluid_ticks.step_tick().is_empty());
    }

    #[test]
    fn chunk_without_y_pos_uses_dimension_minimum() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        // The datafixer writes Y as an int, so the test must too.
        let bytes = test_chunk(vec![test_section(-4, "minecraft:stone", true)]).write();
        let chunk =
            ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("chunk without yPos parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, -64, 0),
            Some(Block::STONE.default_state.id)
        );
    }

    #[test]
    fn chunk_with_int_y_sections_keeps_every_section() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let bytes = test_chunk(vec![
            test_section(-4, "minecraft:stone", true),
            test_section(0, "minecraft:dirt", true),
        ])
        .write();
        let chunk = ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("int Y chunk parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, -64, 0),
            Some(Block::STONE.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::DIRT.default_state.id)
        );
    }

    #[test]
    fn chunk_with_mixed_numeric_y_tags_keeps_sections() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        // What a real upgraded chunk looks like: the map's own sections store Y
        // as a byte, the datafixer writes ints. The int section must still land
        // at its own height instead of collapsing to Y = 0.
        let mut byte_section = test_section(-4, "minecraft:air", true);
        byte_section.put_byte("Y", -4);

        let bytes = test_chunk(vec![
            byte_section,
            test_section(0, "minecraft:stone", true),
            test_section(4, "minecraft:dirt", true),
        ])
        .write();
        let chunk =
            ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("mixed Y chunk parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::STONE.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 64, 0),
            Some(Block::DIRT.default_state.id)
        );
    }

    #[test]
    fn light_only_sections_do_not_determine_allocation() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        // Old worlds keep a light grid at Y = -1 after the upgrade. It has no
        // biomes and must not decide where the chunk starts.
        let mut light_section = NbtCompound::new();
        light_section.put_byte("Y", -1);
        light_section.put(
            "BlockLight",
            NbtTag::ByteArray(vec![0x0Fi8; 2048].into_boxed_slice()),
        );

        let bytes = test_chunk(vec![
            light_section,
            test_section(0, "minecraft:stone", true),
        ])
        .write();
        let chunk =
            ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("light section ignored");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::STONE.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, -1, 0),
            Some(Block::AIR.default_state.id)
        );
    }

    #[test]
    fn sparse_chunk_allocates_the_entire_dimension() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let bytes = test_chunk(vec![test_section(1, "minecraft:stone", true)]).write();
        let chunk = ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("caps at zero");
        // The only section sits at Y=1, so the cap has to bring the chunk down
        // to Y=0. Without the cap the chunk would start at Y=1 and a block at
        // Y=0 would be out of range.
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::AIR.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 16, 0),
            Some(Block::STONE.default_state.id)
        );
    }

    #[test]
    fn chunk_without_y_pos_or_biomes_uses_dimension_bounds() {
        let mut light_section = NbtCompound::new();
        light_section.put_byte("Y", -1);
        let bytes = test_chunk(vec![light_section]).write();
        let chunk = ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).unwrap();
        assert_eq!(
            chunk.section.min_y,
            pumpkin_data::dimension::Dimension::OVERWORLD.min_y
        );
        assert_eq!(
            chunk.section.count as i32 * 16,
            pumpkin_data::dimension::Dimension::OVERWORLD.height
        );
    }

    #[test]
    fn extract_u16_array_from_vanilla_compound_palette() {
        let mut entry1 = NbtCompound::new();
        entry1.put_string("Name", "minecraft:stone".to_string());

        let mut entry2 = NbtCompound::new();
        entry2.put_string("Name", "minecraft:repeater".to_string());
        let mut props = NbtCompound::new();
        props.put_string("facing", "north".to_string());
        props.put_string("delay", "2".to_string());
        props.put_string("locked", "false".to_string());
        props.put_string("powered", "false".to_string());
        entry2.put_compound("Properties", props);

        let list_tag = NbtTag::List(vec![NbtTag::Compound(entry1), NbtTag::Compound(entry2)]);
        let result = extract_u16_array(&list_tag).expect("should extract palette");

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], Block::STONE.default_state.id);

        let repeater_state = Block::REPEATER
            .from_properties(&[
                ("facing", "north"),
                ("delay", "2"),
                ("locked", "false"),
                ("powered", "false"),
            ])
            .to_state_id(&Block::REPEATER);
        assert_eq!(result[1], repeater_state);
    }

    #[test]
    fn extract_u8_array_from_vanilla_string_palette() {
        let list_tag = NbtTag::List(vec![
            NbtTag::String("minecraft:plains".to_string().into()),
            NbtTag::String("minecraft:the_void".to_string().into()),
        ]);
        let result = extract_u8_array(&list_tag).expect("should extract biome palette");

        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            pumpkin_data::biome::Biome::from_name("plains").unwrap().id
        );
        assert_eq!(
            result[1],
            pumpkin_data::biome::Biome::from_name("the_void")
                .unwrap()
                .id
        );
    }
}
