//! PoiRecord.Packed / PoiSection.Packed saved data, without a Minecraft data fixer.
//!
//! Import validates a whole chunk before publishing its position index. Source
//! envelopes retain unknown fields and absent `DataVersion`; only fresh chunks use
//! the supported target version. Rejected images never enter the gameplay index.
//! The existing raw POI I/O owner is unchanged; this is not a publication receipt.

use super::{PoiEntry, PoiRegion};
use crate::world_info::MAXIMUM_SUPPORTED_WORLD_DATA_VERSION;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
use rustc_hash::FxHashSet;
use std::{
    io::{self, Cursor, Read, Write},
    path::Path,
};
use tracing::warn;

const SCHEMA: &str = "murgicraft:poi_schema";
const SCHEMA_VERSION: i32 = 1;
const MAX_CHUNK_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn region_coords(path: &Path) -> io::Result<(i32, i32)> {
    let mut parts = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .split('.');
    let prefix = parts.next();
    let x = parts.next().and_then(|s| s.parse().ok());
    let z = parts.next().and_then(|s| s.parse().ok());
    match (prefix, x, z, parts.next()) {
        (Some("r"), Some(x), Some(z), None) => Ok((x, z)),
        _ => Err(invalid("POI region needs an r.x.z.mca filename")),
    }
}

fn record(record: &NbtCompound) -> io::Result<PoiEntry> {
    // BlockPos.CODEC: canonical presence forbids fallback to fork aliases.
    let (x, y, z) = if record.has("pos") {
        let Some(&[x, y, z]) = record.get_int_array("pos") else {
            return Err(invalid("POI pos must be exactly three ints"));
        };
        (x, y, z)
    } else {
        match (
            record.get_int("x"),
            record.get_int("y"),
            record.get_int("z"),
        ) {
            (Some(x), Some(y), Some(z)) => (x, y, z),
            _ => {
                return Err(invalid(
                    "POI legacy coordinates must be a complete int triple",
                ));
            }
        }
    };
    let poi_type = record
        .get_string("type")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("POI type must be a nonempty string"))?;
    // PoiRecord.Packed loads persisted stock, never the registration maximum.
    let free_tickets = match record.get("free_tickets") {
        None => 0,
        Some(NbtTag::Int(value)) => *value,
        _ => return Err(invalid("POI free_tickets must be an int")),
    };
    Ok(PoiEntry {
        x,
        y,
        z,
        poi_type: poi_type.to_string(),
        free_tickets,
    })
}

fn write_record(mut source: NbtCompound, entry: &PoiEntry) -> NbtTag {
    for alias in ["x", "y", "z"] {
        source.child_tags.remove(alias);
    }
    source.put("pos", NbtTag::IntArray(vec![entry.x, entry.y, entry.z]));
    source.put_string("type", entry.poi_type.clone());
    source.put_int("free_tickets", entry.free_tickets);
    NbtTag::Compound(source)
}

fn section(
    source: &mut NbtCompound,
    section_y: i32,
    index: usize,
    region: (i32, i32),
    seen: &mut FxHashSet<(i32, i32, i32)>,
) -> io::Result<Vec<PoiEntry>> {
    let records = source
        .get_list("Records")
        .ok_or_else(|| invalid("POI Records must be a list"))?;
    let mut canonical = false;
    let mut legacy = false;
    let mut entries = Vec::new();
    let mut normalized = Vec::new();
    for tag in records {
        let raw = tag
            .extract_compound()
            .ok_or_else(|| invalid("POI record must be a compound"))?;
        canonical |= raw.has("pos");
        legacy |= !raw.has("pos");
        let entry = record(raw)?;
        if entry.y >> 4 != section_y
            || (entry.x >> 9, entry.z >> 9) != region
            || PoiRegion::chunk_index(entry.x >> 4, entry.z >> 4) != index
        {
            return Err(invalid(
                "POI position disagrees with containing region/chunk/section",
            ));
        }
        if !seen.insert((entry.x, entry.y, entry.z)) {
            return Err(invalid("duplicate POI position"));
        }
        normalized.push(write_record(raw.clone(), &entry));
        entries.push(entry);
    }
    let valid = match source.get("Valid") {
        Some(NbtTag::Byte(value @ (0 | 1))) => *value,
        None if canonical && legacy => {
            return Err(invalid("mixed POI section has ambiguous Valid default"));
        }
        None => i8::from(legacy), // PoiSection.Packed=false; old fork legacy-only=true.
        _ => return Err(invalid("unsupported POI Valid field")),
    };
    source.put_byte("Valid", valid);
    source.put_list("Records", normalized);
    Ok(entries)
}

fn normalize(source: &mut Nbt, index: usize, region: (i32, i32)) -> io::Result<Vec<PoiEntry>> {
    match source.get(SCHEMA) {
        None | Some(NbtTag::Int(SCHEMA_VERSION)) => {}
        _ => return Err(invalid("unsupported POI schema marker")),
    }
    if source.has("DataVersion") && source.get_int("DataVersion").is_none() {
        return Err(invalid("POI DataVersion must be an int or absent"));
    }
    let Some(NbtTag::Compound(sections)) = source.root_tag.child_tags.get_mut("Sections") else {
        return Err(invalid("POI Sections must be a compound"));
    };
    let mut entries = Vec::new();
    let mut seen = FxHashSet::default();
    for (key, tag) in &mut sections.child_tags {
        let y: i32 = key
            .parse()
            .map_err(|_| invalid("unsupported POI section key"))?;
        if y.to_string() != key.as_ref() {
            return Err(invalid("noncanonical POI section key"));
        }
        let NbtTag::Compound(raw) = tag else {
            return Err(invalid("POI section must be a compound"));
        };
        entries.extend(section(raw, y, index, region, &mut seen)?);
    }
    source.root_tag.put_int(SCHEMA, SCHEMA_VERSION);
    Ok(entries)
}

impl PoiRegion {
    pub(super) fn decompress_chunk_data(
        compressed: &[u8],
        index: usize,
        region: (i32, i32),
    ) -> io::Result<(Nbt, Vec<PoiEntry>, bool)> {
        let mut bytes = Vec::new();
        ZlibDecoder::new(compressed)
            .take(MAX_CHUNK_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_CHUNK_BYTES {
            return Err(invalid("POI chunk exceeds decode limit"));
        }
        let payload_len = bytes.len() as u64;
        // NbtCompound accepts EOF as END. A sentinel makes truncated compounds fail.
        bytes.push(u8::MAX);
        let mut cursor = Cursor::new(&bytes);
        let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            pumpkin_nbt::deserializer::NbtStreamReader(&mut cursor),
        );
        let mut source = Nbt::read(&mut reader).map_err(|e| invalid(&e.to_string()))?;
        if cursor.position() != payload_len {
            return Err(invalid("POI chunk has trailing or incomplete NBT"));
        }
        let original = source.clone();
        let entries = normalize(&mut source, index, region)?;
        let migrated = source != original;
        Ok((source, entries, migrated))
    }

    pub(super) fn compress_chunk_data(source: Nbt) -> io::Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&source.write())?;
        let compressed = encoder.finish()?;
        if (compressed.len() + 5).div_ceil(super::SECTOR_SIZE) > u8::MAX as usize {
            return Err(invalid("POI chunk exceeds MCA sector count"));
        }
        Ok(compressed)
    }

    pub(super) fn get_chunk_data(&self, index: usize) -> Nbt {
        let mut source = self.source_chunks.get(&index).cloned().unwrap_or_else(|| {
            let mut root = NbtCompound::new();
            root.put_int("DataVersion", MAXIMUM_SUPPORTED_WORLD_DATA_VERSION);
            root.put_compound("Sections", NbtCompound::new());
            Nbt::from(root)
        });
        let mut sections = source.get_compound("Sections").cloned().unwrap_or_default();
        for tag in sections.child_tags.values_mut() {
            if let NbtTag::Compound(section) = tag {
                // Keep unknown fields of surviving records by position.
                let old = section.get_list("Records").unwrap_or_default().to_vec();
                let mut records = Vec::new();
                for raw in old {
                    if let NbtTag::Compound(raw) = raw
                        && let Ok(old_entry) = record(&raw)
                        && let Some(entry) = self.entries.get(&Self::pos_key(&old_entry.pos()))
                    {
                        records.push(write_record(raw, entry));
                    }
                }
                section.put_list("Records", records);
            }
        }
        for entry in self
            .entries
            .values()
            .filter(|e| Self::chunk_index(e.x >> 4, e.z >> 4) == index)
        {
            let key = Self::section_key(&entry.pos());
            let tag = sections.child_tags.entry(key.into()).or_insert_with(|| {
                let mut section = NbtCompound::new();
                section.put_byte("Valid", 1);
                section.put_list("Records", Vec::new());
                NbtTag::Compound(section)
            });
            if let NbtTag::Compound(section) = tag
                && let Some(NbtTag::List(records)) = section.child_tags.get_mut("Records")
                && !records.iter().any(|raw| {
                    raw.extract_compound().and_then(|r| r.get_int_array("pos"))
                        == Some(&[entry.x, entry.y, entry.z])
                })
            {
                records.push(write_record(NbtCompound::new(), entry));
            }
        }
        source.root_tag.put_compound("Sections", sections);
        source.root_tag.put_int(SCHEMA, SCHEMA_VERSION);
        source
    }

    pub(super) fn mutation_allowed(&self, index: usize) -> bool {
        if let Some(reason) = self.unavailable.as_deref().or_else(|| {
            self.retained_chunks
                .get(&index)
                .map(|(_, reason)| reason.as_str())
        }) {
            warn!("POI mutation refused until repair: {reason}");
            return false;
        }
        true
    }

    pub(super) fn check_preservation(&self) -> io::Result<()> {
        if let Some(reason) = &self.unavailable {
            return Err(invalid(&format!(
                "POI region unavailable; original file retained: {reason}"
            )));
        }
        if let Some((index, (image, reason))) = self.retained_chunks.iter().next() {
            return Err(invalid(&format!(
                "POI chunk {index} retained ({} bytes); region save refused until repair: {reason}",
                image.len()
            )));
        }
        Ok(())
    }
}
