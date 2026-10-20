//! Literal fixtures deliberately bypass the production POI encoder.
#![expect(clippy::unwrap_used, reason = "isolated literal storage fixtures")]

use super::{PoiRegion, PoiStorage};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
};

const HOME: &str = "minecraft:home";
const PORTAL: &str = "minecraft:nether_portal";
const STATION: &str = "minecraft:librarian";

fn pos(x: i32, y: i32, z: i32) -> BlockPos {
    BlockPos(Vector3::new(x, y, z))
}

fn canonical(p: [i32; 3], kind: &str, tickets: i32) -> NbtCompound {
    let mut record = NbtCompound::new();
    record.put("pos", NbtTag::IntArray(p.to_vec()));
    record.put_string("type", kind.to_string());
    record.put_int("free_tickets", tickets);
    record
}

fn legacy(p: [i32; 3], kind: &str, tickets: i32) -> NbtCompound {
    let mut record = canonical(p, kind, tickets);
    record.child_tags.remove("pos");
    for (key, value) in ["x", "y", "z"].into_iter().zip(p) {
        record.put_int(key, value);
    }
    record
}

fn root(version: Option<i32>, sections: Vec<(&str, Option<i8>, Vec<NbtCompound>)>) -> NbtCompound {
    let mut source = NbtCompound::new();
    if let Some(version) = version {
        source.put_int("DataVersion", version);
    }
    let mut envelope = NbtCompound::new();
    for (key, valid, records) in sections {
        let mut section = NbtCompound::new();
        if let Some(valid) = valid {
            section.put_byte("Valid", valid);
        }
        section.put_list(
            "Records",
            records.into_iter().map(NbtTag::Compound).collect(),
        );
        envelope.put_compound(key, section);
    }
    source.put_compound("Sections", envelope);
    source
}

fn write_chunk(path: &Path, index: usize, source: NbtCompound) {
    write_mca(path, &[(index, source)]);
}

fn write_mca(path: &Path, chunks: &[(usize, NbtCompound)]) {
    let images: Vec<_> = chunks
        .iter()
        .map(|(index, root)| {
            (
                *index,
                Nbt::new("fixture".to_string(), root.clone())
                    .write()
                    .to_vec(),
            )
        })
        .collect();
    write_images(path, &images);
}

fn write_images(path: &Path, chunks: &[(usize, Vec<u8>)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = vec![0; 8192];
    for (index, raw) in chunks {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let offset = bytes.len() / 4096;
        let count = (compressed.len() + 5).div_ceil(4096);
        let location = ((offset as u32) << 8) | count as u32;
        bytes[index * 4..index * 4 + 4].copy_from_slice(&location.to_be_bytes());
        bytes.extend_from_slice(&((compressed.len() + 1) as u32).to_be_bytes());
        bytes.push(2);
        bytes.extend_from_slice(&compressed);
        bytes.resize((offset + count) * 4096, 0);
    }
    std::fs::write(path, bytes).unwrap();
}

fn read_chunk(path: &Path, index: usize) -> Nbt {
    let bytes = std::fs::read(path).unwrap();
    let location = u32::from_be_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    let offset = (location >> 8) as usize * 4096;
    let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    assert_eq!(bytes[offset + 4], 2);
    let mut raw = Vec::new();
    ZlibDecoder::new(&bytes[offset + 5..offset + 4 + length])
        .read_to_end(&mut raw)
        .unwrap();
    Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
        pumpkin_nbt::deserializer::NbtStreamReader(&mut Cursor::new(raw)),
    ))
    .unwrap()
}

fn entries(path: &Path) -> Vec<(i32, i32, i32, String, i32)> {
    let region = PoiRegion::load(path).unwrap();
    let mut entries: Vec<_> = region
        .get_all()
        .into_iter()
        .map(|r| (r.x, r.y, r.z, r.poi_type.clone(), r.free_tickets))
        .collect();
    entries.sort();
    entries
}

fn save_region(path: &Path) {
    let mut region = PoiRegion::load(path).unwrap();
    if let Some(entry) = region.get_all().first().map(|entry| (**entry).clone()) {
        region.add(entry); // Force the real writer even after migration is idempotent.
    }
    region.save(path).unwrap();
}

fn assert_record(source: &Nbt, section: &str, p: [i32; 3], kind: &str, tickets: i32) {
    let section = source
        .get_compound("Sections")
        .unwrap()
        .get_compound(section)
        .unwrap();
    let found = section
        .get_list("Records")
        .unwrap()
        .iter()
        .filter_map(NbtTag::extract_compound)
        .find(|r| r.get_int_array("pos") == Some(p.as_slice()));
    assert!(found.is_some(), "saved canonical position/type envelope");
    let record = found.unwrap();
    assert_eq!(
        record.get_int_array("pos"),
        Some(p.as_slice()),
        "canonical pos int-array"
    );
    assert_eq!(record.get_string("type"), Some(kind));
    assert_eq!(
        record.get_int("free_tickets"),
        Some(tickets),
        "literal ticket count"
    );
    assert!(
        !record.has("x") && !record.has("y") && !record.has("z"),
        "no legacy aliases"
    );
}

#[test]
fn villager_s4_vanilla_positions_and_tickets_survive_two_storage_reloads() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("poi");
    let positive = folder.join("r.0.0.mca");
    let negative = folder.join("r.-1.-1.mca");
    let fixtures = [
        ([1, 64, 2], HOME, 0),
        ([2, 80, 3], HOME, 1),
        ([3, 96, 4], PORTAL, 17),
        ([-1, -17, -2], STATION, -23),
    ];
    write_chunk(
        &positive,
        0,
        root(
            Some(5023),
            vec![
                (
                    "4",
                    Some(1),
                    vec![canonical(fixtures[0].0, fixtures[0].1, 0)],
                ),
                (
                    "5",
                    Some(0),
                    vec![canonical(fixtures[1].0, fixtures[1].1, 1)],
                ),
                (
                    "6",
                    Some(1),
                    vec![canonical(fixtures[2].0, fixtures[2].1, 17)],
                ),
            ],
        ),
    );
    write_chunk(
        &negative,
        1023,
        root(
            Some(5023),
            vec![(
                "-2",
                Some(0),
                vec![canonical(fixtures[3].0, fixtures[3].1, -23)],
            )],
        ),
    );
    for _ in 0..3 {
        let mut storage = PoiStorage::new(folder.clone());
        assert_eq!(
            storage.get_in_square(pos(0, 64, 0), 16, None).len(),
            4,
            "canonical locations/count"
        );
        for (p, kind, tickets) in fixtures {
            assert!(
                entries(if p[0] < 0 { &negative } else { &positive }).contains(&(
                    p[0],
                    p[1],
                    p[2],
                    kind.to_string(),
                    tickets
                )),
                "exact imported entry"
            );
            storage.add_with_free_tickets(pos(p[0], p[1], p[2]), kind, tickets);
        }
        storage.save_all().unwrap();
        for (p, kind, tickets) in fixtures {
            let source = read_chunk(
                if p[0] < 0 { &negative } else { &positive },
                if p[0] < 0 { 1023 } else { 0 },
            );
            assert_record(&source, &(p[1] >> 4).to_string(), p, kind, tickets);
        }
    }
}

#[test]
fn villager_s4_full_width_coordinates_and_missing_version() {
    let dir = tempfile::tempdir().unwrap();
    // Full-width positions avoid the facade's existing query-radius arithmetic.
    let extremes = dir.path().join("r.-4194304.4194303.mca");
    write_chunk(
        &extremes,
        992,
        root(
            None,
            vec![(
                "-134217728",
                None,
                vec![canonical(
                    [i32::MIN, i32::MIN, i32::MAX],
                    "custom:extreme",
                    i32::MIN,
                )],
            )],
        ),
    );
    for _ in 0..2 {
        save_region(&extremes);
        assert_eq!(
            entries(&extremes),
            vec![(
                i32::MIN,
                i32::MIN,
                i32::MAX,
                "custom:extreme".to_string(),
                i32::MIN
            )]
        );
        assert_eq!(read_chunk(&extremes, 992).get_int("DataVersion"), None);
    }
}

#[test]
fn villager_s4_legacy_coordinates_migrate_once_without_ticket_reset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.0.0.mca");
    let mut precedence = canonical([2, 80, 2], "custom:station", 27);
    for key in ["x", "y", "z"] {
        precedence.put_int(key, 0);
    }
    let mut missing_tickets = legacy([3, 96, 3], HOME, 99);
    missing_tickets.child_tags.remove("free_tickets");
    write_chunk(
        &path,
        0,
        root(
            Some(3955),
            vec![
                ("4", Some(0), vec![legacy([1, 64, 1], HOME, 0)]),
                (
                    "5",
                    Some(1),
                    vec![precedence, legacy([5, 80, 5], PORTAL, 11)],
                ),
                ("6", None, vec![missing_tickets]),
                ("7", None, vec![canonical([4, 112, 4], HOME, 1)]),
            ],
        ),
    );
    let expected = vec![
        (1, 64, 1, HOME.to_string(), 0),
        (2, 80, 2, "custom:station".to_string(), 27),
        (3, 96, 3, HOME.to_string(), 0),
        (4, 112, 4, HOME.to_string(), 1),
        (5, 80, 5, PORTAL.to_string(), 11),
    ];
    assert_eq!(
        entries(&path),
        expected,
        "canonical precedence and literal/default tickets"
    );
    save_region(&path);
    let migrated = read_chunk(&path, 0);
    for _ in 0..2 {
        assert_eq!(
            entries(&path),
            expected,
            "tickets must not reset during hydrate"
        );
        save_region(&path);
        let source = read_chunk(&path, 0);
        assert_eq!(
            source.get_int("murgicraft:poi_schema"),
            Some(1),
            "migration marker"
        );
        assert_eq!(source.get_int("DataVersion"), Some(3955), "source version");
        let sections = source.get_compound("Sections").unwrap();
        for (key, valid) in [("4", 0), ("5", 1), ("6", 1), ("7", 0)] {
            assert_eq!(
                sections.get_compound(key).unwrap().get_byte("Valid"),
                Some(valid),
                "explicit Valid default {key}"
            );
        }
        for (x, y, z, kind, tickets) in &expected {
            assert_record(&source, &(y >> 4).to_string(), [*x, *y, *z], kind, *tickets);
        }
        assert_eq!(source, migrated, "second canonical payload is idempotent");
    }
}

fn malformed_records() -> Vec<Vec<NbtCompound>> {
    let base = canonical([1, 64, 1], HOME, 0);
    let mut cases = Vec::new();
    for length in [0, 1, 2, 4] {
        let mut record = base.clone();
        record.put("pos", NbtTag::IntArray(vec![1; length]));
        for key in ["x", "y", "z"] {
            record.put_int(key, 0);
        }
        cases.push(vec![record]);
    }
    for (key, tag) in [
        ("pos", NbtTag::Int(1)),
        ("type", NbtTag::Int(1)),
        ("type", NbtTag::String("".into())),
        ("free_tickets", NbtTag::Short(0)),
    ] {
        let mut record = base.clone();
        record.put(key, tag);
        cases.push(vec![record]);
    }
    let mut partial = legacy([1, 64, 1], HOME, 0);
    partial.child_tags.remove("x");
    cases.push(vec![partial]);
    let mut missing_type = base.clone();
    missing_type.child_tags.remove("type");
    cases.push(vec![missing_type]);
    cases.push(vec![base.clone(), base]);
    for p in [[1, 80, 1], [17, 64, 1], [513, 64, 1]] {
        cases.push(vec![canonical(p, HOME, 0)]);
    }
    cases
}

#[test]
fn villager_s4_malformed_positions_cannot_overwrite_original_poi() {
    for (case, records) in malformed_records().into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.0.0.mca");
        write_mca(
            &path,
            &[
                (0, root(Some(5023), vec![("4", Some(1), records)])),
                (
                    1,
                    root(
                        Some(5023),
                        vec![("4", Some(1), vec![canonical([17, 64, 1], STATION, 6)])],
                    ),
                ),
            ],
        );
        let original = std::fs::read(&path).unwrap();
        for _ in 0..2 {
            let mut region = PoiRegion::load(&path).unwrap();
            assert_eq!(
                region.get_all().len(),
                1,
                "case {case}: malformed records must not enter index"
            );
            assert_eq!(
                region.get_all()[0].pos(),
                pos(17, 64, 1),
                "valid neighbor available"
            );
            let result = region.save(&path);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original,
                "retained source cannot be overwritten"
            );
            assert!(
                result.is_err(),
                "normal save must report preservation refusal"
            );
            let error = result.unwrap_err();
            region.add(super::PoiEntry::new_portal(pos(0, 0, 0)));
            assert_eq!(region.get_all().len(), 1, "affected chunk rejects add");
            assert!(
                !region.remove(&pos(1, 64, 1)),
                "affected chunk mutation disabled"
            );

            assert!(
                error.to_string().contains("retained"),
                "observable preservation diagnostic"
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original,
                "case {case}: original bytes survive refusal"
            );
            let mut storage = PoiStorage::new(dir.path().to_path_buf());
            assert_eq!(
                storage.get_in_square(pos(16, 64, 0), 32, None),
                vec![pos(17, 64, 1)]
            );
            storage.add_portal(pos(18, 64, 1)); // Unaffected chunk still admits ordinary edits.
            assert_eq!(storage.get_in_square(pos(16, 64, 0), 32, None).len(), 2);
            assert!(
                storage.save_all().is_err(),
                "facade must report retained data"
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original,
                "facade cannot overwrite retained bytes"
            );
        }
    }
}

#[test]
fn villager_s4_section_envelope_and_unknown_data_survive_real_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.-1.-1.mca");
    let mut record = canonical([-1, -17, -1], "extension:unknown", 42);
    record.put_string("extension:record", "stock".to_string());
    let mut source = root(
        Some(9876),
        vec![("-2", Some(0), vec![record]), ("-3", Some(0), vec![])],
    );
    source.put(
        "extension:root",
        NbtTag::LongArray(vec![i64::MIN, i64::MAX]),
    );
    if let Some(NbtTag::Compound(sections)) = source.child_tags.get_mut("Sections")
        && let Some(NbtTag::Compound(section)) = sections.child_tags.get_mut("-2")
    {
        section.put_string("extension:section", "keep".to_string());
    }
    write_mca(&path, &[(1023, source.clone())]);
    for _ in 0..2 {
        save_region(&path);
        let saved = read_chunk(&path, 1023);
        assert_eq!(saved.name, "fixture");
        assert_eq!(
            saved.get_int("DataVersion"),
            Some(9876),
            "never relabel imported data"
        );
        assert_eq!(
            saved.get("extension:root"),
            source.get("extension:root"),
            "unknown root data"
        );
        assert_eq!(
            saved.get_compound("Sections"),
            source.get_compound("Sections"),
            "validity, empty sections and unknown raw fields"
        );
        assert_eq!(saved.get_int("murgicraft:poi_schema"), Some(1));
    }
    let mut future = root(
        Some(9876),
        vec![("4", Some(1), vec![canonical([1, 64, 1], HOME, 0)])],
    );
    future.put_int("murgicraft:poi_schema", 9);
    let mixed = root(
        None,
        vec![(
            "4",
            None,
            vec![canonical([1, 64, 1], HOME, 0), legacy([2, 64, 2], HOME, 1)],
        )],
    );
    for unsupported in [future, mixed] {
        let path = dir.path().join("r.0.0.mca");
        write_mca(&path, &[(0, unsupported.clone())]);
        let original = std::fs::read(&path).unwrap();
        for _ in 0..2 {
            let mut region = PoiRegion::load(&path).unwrap();
            let result = region.save(&path);
            assert_eq!(
                read_chunk(&path, 0).get("murgicraft:poi_schema"),
                unsupported.get("murgicraft:poi_schema"),
                "no false success marker"
            );
            assert!(result.is_err(), "unsupported envelope refuses migration");
            assert!(region.get_all().is_empty());
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original,
                "unsupported source bytes"
            );
        }
    }
    let empty = dir.path().join("r.1.1.mca");
    write_mca(&empty, &[(0, root(None, vec![("-4", None, vec![])]))]);
    for _ in 0..2 {
        let mut region = PoiRegion::load(&empty).unwrap();
        region.add(super::PoiEntry::new_portal(pos(512, -64, 512)));
        assert!(region.remove(&pos(512, -64, 512)));
        region.save(&empty).unwrap();
        assert!(empty.exists(), "empty source chunks survive");
        assert_eq!(read_chunk(&empty, 0).get_int("DataVersion"), None);
        assert_eq!(
            read_chunk(&empty, 0)
                .get_compound("Sections")
                .unwrap()
                .get_compound("-4")
                .unwrap()
                .get_byte("Valid"),
            Some(0)
        );
    }
}

#[test]
fn villager_s4_storage_facade_reload_keeps_neighboring_record_types() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("poi");
    let path = folder.join("r.0.0.mca");
    write_chunk(
        &path,
        0,
        root(
            Some(5023),
            vec![(
                "4",
                Some(1),
                vec![
                    canonical([1, 64, 1], STATION, 5),
                    canonical([2, 64, 2], HOME, 0),
                    canonical([3, 64, 3], PORTAL, 7),
                ],
            )],
        ),
    );
    for _ in 0..3 {
        // Initial hydration plus two genuinely fresh facade reloads.
        let mut storage = PoiStorage::new(folder.clone());
        assert_eq!(
            storage.get_in_square(pos(0, 64, 0), 16, Some(HOME)),
            vec![pos(2, 64, 2)],
            "HOME location"
        );
        assert_eq!(
            storage.find_closest_matching(pos(0, 64, 0), 16, |t| t == STATION),
            Some((pos(1, 64, 1), STATION.to_string()))
        );
        assert_eq!(
            storage.get_in_square(pos(0, 64, 0), 16, Some(PORTAL)),
            vec![pos(3, 64, 3)]
        );
        storage.add(pos(4, 64, 4), "custom:unrelated");
        assert!(storage.remove(&pos(4, 64, 4)));
        storage.save_all().unwrap();
        let source = read_chunk(&path, 0);
        for (p, kind, tickets) in [
            ([1, 64, 1], STATION, 5),
            ([2, 64, 2], HOME, 0),
            ([3, 64, 3], PORTAL, 7),
        ] {
            assert_record(&source, "4", p, kind, tickets);
        }
        assert_eq!(entries(&path).len(), 3);
    }
}

#[test]
fn villager_s4_failed_region_load_cannot_be_saved_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.0.0.mca");
    for bytes in [vec![0; 12], vec![255; 8192]] {
        std::fs::write(&path, &bytes).unwrap();
        for _ in 0..2 {
            let mut storage = PoiStorage::new(dir.path().to_path_buf());
            assert!(storage.get_in_square(pos(0, 0, 0), 1, None).is_empty());
            storage.add_portal(pos(0, 0, 0));
            let result = storage.save_all();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                bytes,
                "failed load cannot overwrite source"
            );
            assert!(result.is_err(), "failed load must stay unavailable");
            assert!(
                storage.get_in_square(pos(0, 0, 0), 1, None).is_empty(),
                "unavailable region rejects edits"
            );
        }
    }
    // A known bad MCA slot must not hide a valid neighboring chunk.
    write_chunk(
        &path,
        1,
        root(
            Some(5023),
            vec![("4", Some(1), vec![canonical([17, 64, 1], STATION, 6)])],
        ),
    );
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[..4].copy_from_slice(&0xffff_ff01u32.to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();
    for _ in 0..2 {
        let mut storage = PoiStorage::new(dir.path().to_path_buf());
        assert_eq!(
            storage.get_in_square(pos(16, 64, 0), 32, None),
            vec![pos(17, 64, 1)],
            "bad MCA slot keeps neighbor available"
        );
        assert!(storage.save_all().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn villager_s4_incomplete_nbt_keeps_original_chunk_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.0.0.mca");
    let raw = Nbt::from(root(None, vec![])).write();
    for missing_ends in [1, 2] {
        write_images(&path, &[(0, raw[..raw.len() - missing_ends].to_vec())]);
        let original = std::fs::read(&path).unwrap();
        for _ in 0..2 {
            let mut region = PoiRegion::load(&path).unwrap();
            assert!(region.get_all().is_empty());
            assert!(
                region.save(&path).is_err(),
                "incomplete NBT must refuse migration"
            );
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
    }
}
