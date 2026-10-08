//! Entity and light storage used by `WorldGenRegion`'s spawn stage.

use crate::chunk::{ChunkData, ChunkLight};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::vector3::Vector3;

pub(super) fn read_generated_entities(chunk: &ChunkData) -> Vec<NbtCompound> {
    chunk
        .get_custom_data("murgicraft", "generated_entities")
        .and_then(|tag| tag.extract_list().map(<[_]>::to_vec))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tag| tag.extract_compound().cloned())
        .collect()
}

/// Reads a generation chunk's light at an absolute block position, with `LevelReader` height defaults.
#[must_use]
pub fn light_at(light: &ChunkLight, min_y: i32, pos: &Vector3<i32>, sky: bool) -> u8 {
    let y = pos.y - min_y;
    let sections = if sky {
        &light.sky_light
    } else {
        &light.block_light
    };
    usize::try_from(y)
        .ok()
        .and_then(|y| {
            sections
                .get(y / 16)
                .map(|section| section.get((pos.x & 15) as usize, y & 15, (pos.z & 15) as usize))
        })
        .unwrap_or(if sky { 15 } else { 0 })
}

pub(crate) fn store_generated_entities(chunk: &ChunkData, entities: Vec<NbtCompound>) {
    if !entities.is_empty() {
        chunk.set_custom_data(
            "murgicraft",
            "generated_entities",
            NbtTag::List(entities.into_iter().map(NbtTag::Compound).collect()),
        );
    }
}
