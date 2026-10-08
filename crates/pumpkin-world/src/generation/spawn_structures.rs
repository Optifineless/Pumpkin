//! `StructureManager` spawn bounds, retained from real starts/references through chunk publication.
use super::proto_chunk::ProtoChunk;
use crate::{chunk::ChunkData, generation::structure::structures::StructurePiecesCollector};
use pumpkin_data::structures::{Structure, StructureKeys, TerrainAdaptation};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::block_box::BlockBox;
use std::sync::{Arc, Mutex};

pub(crate) type SpawnStructureReference = (StructureKeys, Arc<Mutex<StructurePiecesCollector>>);

impl ProtoChunk {
    pub(crate) fn refresh_spawn_structure_bounds(&mut self) {
        // Structure pieces such as SwampHutPiece can move during postProcess.
        if !self.spawn_structure_references.is_empty() {
            self.spawn_structures = self
                .spawn_structure_references
                .iter()
                .filter_map(|(key, collector)| {
                    from_collector(
                        *key,
                        &mut collector
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    )
                })
                .collect();
        }
    }
}

pub(crate) fn adjusted_bounds(structure: &Structure, bounds: BlockBox) -> BlockBox {
    // Structure.adjustBoundingBox (Structure.java:80-81), used by StructureStart.getBoundingBox.
    const TERRAIN_ADAPTATION_PADDING: i32 = 12;
    if structure.terrain_adaptation == TerrainAdaptation::None {
        bounds
    } else {
        bounds.expand(
            TERRAIN_ADAPTATION_PADDING,
            TERRAIN_ADAPTATION_PADDING,
            TERRAIN_ADAPTATION_PADDING,
        )
    }
}

/// Actual generated structure bounds intersecting a chunk, supplied by structure storage.
#[derive(Clone)]
pub struct StructureSpawnBounds {
    pub structure: StructureKeys,
    pub full: BlockBox,
    pub pieces: Vec<BlockBox>,
}

/// Records the starts referenced by this chunk for the runtime spawn resolver.
/// Call after generating/loading structure metadata; include referenced starts in other chunks.
pub fn record_structure_spawn_bounds(chunk: &ChunkData, bounds: &[StructureSpawnBounds]) {
    fn box_tag(bounds: &BlockBox) -> NbtTag {
        NbtTag::IntArray(vec![
            bounds.min.x,
            bounds.min.y,
            bounds.min.z,
            bounds.max.x,
            bounds.max.y,
            bounds.max.z,
        ])
    }
    let entries = bounds
        .iter()
        .map(|bounds| {
            let mut entry = NbtCompound::new();
            entry.put_string("id", bounds.structure.to_name().to_string());
            entry.put("BB", box_tag(&bounds.full));
            entry.put(
                "pieces",
                NbtTag::List(bounds.pieces.iter().map(box_tag).collect()),
            );
            NbtTag::Compound(entry)
        })
        .collect();
    chunk.set_custom_data("murgicraft", "spawn_structures", NbtTag::List(entries));
}

/// Restores the generated structure bounds saved with a chunk.
pub fn structure_bounds_from_chunk(chunk: &ChunkData) -> Vec<StructureSpawnBounds> {
    structure_bounds_from_data(
        chunk
            .get_custom_data("murgicraft", "spawn_structures")
            .as_ref(),
    )
}

pub fn structure_bounds_from_data(data: Option<&NbtTag>) -> Vec<StructureSpawnBounds> {
    fn read_box(tag: &NbtTag) -> Option<BlockBox> {
        let a = tag.extract_int_array()?;
        (a.len() == 6).then(|| BlockBox::new(a[0], a[1], a[2], a[3], a[4], a[5]))
    }
    let mut bounds = Vec::new();
    if let Some(entries) = data.and_then(NbtTag::extract_list) {
        for tag in entries {
            if let Some(entry) = tag.extract_compound()
                && let Some(structure) = entry.get_string("id").and_then(StructureKeys::from_name)
                && let Some(full) = entry.get("BB").and_then(read_box)
            {
                let pieces = entry
                    .get_list("pieces")
                    .map_or_else(Vec::new, |list| list.iter().filter_map(read_box).collect());
                bounds.push(StructureSpawnBounds {
                    structure,
                    full,
                    pieces,
                });
            }
        }
    }
    bounds
}

/// Snapshots a real generated start or reference for spawn overrides and `SpawnContext` structure checks.
pub(crate) fn from_collector(
    structure: StructureKeys,
    collector: &mut StructurePiecesCollector,
) -> Option<StructureSpawnBounds> {
    if Structure::get(&structure).spawn_overrides.is_empty() || collector.pieces.is_empty() {
        return None;
    }
    Some(StructureSpawnBounds {
        structure,
        full: adjusted_bounds(Structure::get(&structure), collector.get_bounding_box()),
        pieces: collector
            .pieces
            .iter()
            .map(|piece| piece.bounding_box())
            .collect(),
    })
}
