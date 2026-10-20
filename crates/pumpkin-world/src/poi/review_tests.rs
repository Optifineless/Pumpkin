//! Rollback decoder copied from b48caaf06 and cross-region facade regressions.
use super::*;
use crate::poi::{POI_TYPE_NETHER_PORTAL, PoiChunkData, PoiEntry, PoiSectionData};
use rustc_hash::FxHashMap;

// b48caaf06 used this fallback even though imported versions may differ.
const DATA_VERSION: i32 = 3955;

/// Decompress chunk data from bytes
fn baseline_decompress_chunk_data(compressed: &[u8]) -> std::io::Result<PoiChunkData> {
    let mut decoder = ZlibDecoder::new(compressed);
    let mut uncompressed = Vec::new();
    decoder.read_to_end(&mut uncompressed)?;

    let mut cursor = Cursor::new(uncompressed);
    let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(
        pumpkin_nbt::deserializer::NbtStreamReader(&mut cursor),
    );
    let nbt = pumpkin_nbt::Nbt::read(&mut reader)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let data_version = nbt.get_int("DataVersion").unwrap_or(DATA_VERSION);
    let mut sections = FxHashMap::default();

    if let Some(sec_tag) = nbt.get_compound("Sections") {
        for (sec_key, tag) in &sec_tag.child_tags {
            if let pumpkin_nbt::tag::NbtTag::Compound(sec_comp) = tag {
                let valid = sec_comp.get_byte("Valid").unwrap_or(1);
                let mut records = Vec::new();
                if let Some(pumpkin_nbt::tag::NbtTag::List(rec_list)) = sec_comp.get("Records") {
                    for rec_t in rec_list {
                        if let pumpkin_nbt::tag::NbtTag::Compound(rc) = rec_t {
                            records.push(PoiEntry {
                                x: rc.get_int("x").unwrap_or(0),
                                y: rc.get_int("y").unwrap_or(0),
                                z: rc.get_int("z").unwrap_or(0),
                                poi_type: rc
                                    .get_string("type")
                                    .unwrap_or(POI_TYPE_NETHER_PORTAL)
                                    .to_string(),
                                free_tickets: rc.get_int("free_tickets").unwrap_or(0),
                            });
                        }
                    }
                }
                sections.insert(sec_key.to_string(), PoiSectionData { valid, records });
            }
        }
    }

    Ok(PoiChunkData {
        data_version,
        sections,
    })
}

fn baseline_load(path: &Path) -> Vec<(i32, i32, i32, String, i32)> {
    let bytes = std::fs::read(path).unwrap();
    let mut entries = FxHashMap::default();
    for index in 0..1024 {
        let location = u32::from_be_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
        if location == 0 {
            continue;
        }
        let offset = (location >> 8) as usize * 4096;
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        assert_eq!(bytes[offset + 4], 2);
        let chunk_data =
            baseline_decompress_chunk_data(&bytes[offset + 5..offset + 4 + length]).unwrap();
        // b48caaf06 PoiRegion::load:409-412, including coordinate collisions.
        for section in chunk_data.sections.values() {
            for entry in &section.records {
                let key = (entry.x, entry.y, entry.z);
                entries.insert(key, entry.clone());
            }
        }
    }
    let mut entries: Vec<_> = entries
        .into_values()
        .map(|r| (r.x, r.y, r.z, r.poi_type, r.free_tickets))
        .collect();
    entries.sort();
    entries
}

#[test]
fn villager_s4_new_saves_load_on_rollback_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("poi");
    let path = folder.join("r.0.0.mca");
    let expected = vec![
        (1, 64, 1, HOME.to_string(), 0),
        (2, 64, 2, PORTAL.to_string(), 17),
        (3, 64, 3, STATION.to_string(), -6),
    ];
    let source = root(
        Some(5023),
        vec![(
            "4",
            Some(1),
            vec![
                canonical([1, 64, 1], HOME, 0),
                legacy([2, 64, 2], PORTAL, 17),
                canonical([3, 64, 3], STATION, -6),
            ],
        )],
    );
    write_chunk(&path, 0, source);
    for pass in 0..3 {
        let mut storage = PoiStorage::new(folder.clone());
        assert_eq!(storage.get_in_square(pos(1, 64, 1), 16, None).len(), 3);
        if pass > 0 {
            // Exercise ordinary edits as well as hydration-only migration.
            assert!(storage.remove(&pos(3, 64, 3)));
            storage.add_with_free_tickets(pos(3, 64, 3), STATION, -6);
        }
        storage.save_all().unwrap();
        assert_eq!(baseline_load(&path), expected, "rollback pass {pass}");
        assert_eq!(entries(&path), expected, "current reader pass {pass}");
    }
    let fresh_folder = dir.path().join("fresh");
    let mut fresh = PoiStorage::new(fresh_folder.clone());
    for (x, y, z, kind, tickets) in &expected {
        fresh.add_with_free_tickets(pos(*x, *y, *z), kind, *tickets);
    }
    fresh.save_all().unwrap();
    assert_eq!(baseline_load(&fresh_folder.join("r.0.0.mca")), expected);

    // The actual old decoder must detect the original canonical-only failure.
    let control = dir.path().join("canonical-only/r.0.0.mca");
    write_chunk(
        &control,
        0,
        root(
            Some(5023),
            vec![(
                "4",
                Some(1),
                expected
                    .iter()
                    .map(|r| canonical([r.0, r.1, r.2], &r.3, r.4))
                    .collect(),
            )],
        ),
    );
    let collapsed = baseline_load(&control);
    assert_eq!(
        collapsed.len(),
        1,
        "old decoder collapses canonical-only records"
    );
    assert_eq!((collapsed[0].0, collapsed[0].1, collapsed[0].2), (0, 0, 0));
}

fn region_position(key: (i32, i32), local_x: i32) -> BlockPos {
    pos(key.0 * 512 + local_x, 64, key.1 * 512 + 1)
}

fn corrupt_middle(folder: &Path, key: (i32, i32), retained: bool) -> Vec<u8> {
    let path = folder.join(format!("r.{}.{}.mca", key.0, key.1));
    if retained {
        let p = region_position(key, 1);
        let mut malformed = canonical([p.0.x, p.0.y, p.0.z], HOME, 0);
        malformed.put("pos", NbtTag::IntArray(vec![p.0.x, p.0.y]));
        write_mca(
            &path,
            &[
                (0, root(Some(5023), vec![("4", Some(1), vec![malformed])])),
                (
                    1,
                    root(
                        Some(5023),
                        vec![(
                            "4",
                            Some(1),
                            vec![canonical([p.0.x + 16, p.0.y, p.0.z], STATION, 6)],
                        )],
                    ),
                ),
            ],
        );
    } else {
        std::fs::write(&path, [0; 12]).unwrap();
    }
    std::fs::read(path).unwrap()
}

#[test]
fn villager_s4_bad_middle_region_does_not_block_healthy_saves() {
    for retained in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("poi");
        let insertion_order = [(0, 0), (0, 1), (0, 2)];
        let mut seed = PoiStorage::new(folder.clone());
        for key in insertion_order {
            seed.add_portal(region_position(key, 1));
        }
        seed.save_all().unwrap();
        // Observe the real map, then reproduce its insertion order after restart.
        let save_order: Vec<_> = seed.regions.keys().copied().collect();
        drop(seed);
        let middle = save_order[1];
        let original = corrupt_middle(&folder, middle, retained);
        let bad_path = folder.join(format!("r.{}.{}.mca", middle.0, middle.1));
        let mut storage = PoiStorage::new(folder.clone());
        for key in insertion_order {
            storage.get_in_square(region_position(key, 1), 0, None);
        }
        assert_eq!(
            storage.regions.keys().copied().collect::<Vec<_>>(),
            save_order
        );
        if retained {
            // A valid neighboring chunk admits edits, but its region cannot save.
            storage.add_portal(region_position(middle, 18));
        }
        let healthy = [save_order[0], save_order[2]];
        for local_x in [2, 3] {
            for key in healthy {
                assert!(storage.remove(&region_position(key, local_x - 1)));
                storage.add_portal(region_position(key, local_x));
            }
            let error = storage.save_all().unwrap_err().to_string();
            assert_eq!(std::fs::read(&bad_path).unwrap(), original);
            let mut restarted = PoiStorage::new(folder.clone());
            for key in healthy {
                let current = region_position(key, local_x);
                assert_eq!(
                    restarted.get_in_square(current, 8, Some(PORTAL)),
                    vec![current]
                );
                assert!(!storage.regions[&key].is_dirty());
            }
            assert!(error.contains(bad_path.to_str().unwrap()));
            assert!(error.contains("retained"));
            assert_eq!(storage.regions[&middle].is_dirty(), retained);
        }
    }
}
