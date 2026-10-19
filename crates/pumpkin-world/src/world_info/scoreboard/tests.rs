use std::{fs, io::Write};

use flate2::{Compression, write::GzEncoder};
use pumpkin_nbt::{Nbt, tag::NbtTag};

use super::*;

#[test]
fn scoreboard_missing_file_is_empty_without_creating_a_document() {
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(read_scoreboard(folder.path()).unwrap(), None);
    assert!(!minecraft_data_dir(folder.path()).exists());
}

#[test]
fn scoreboard_saved_data_round_trip_preserves_raw_payload() {
    let folder = tempfile::tempdir().unwrap();
    // ScoreboardSaveData.Packed.CODEC stores four optional sections directly in data.
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "custom".into());
    objective.put_string("CriteriaName", "minecraft.custom:minecraft.jump".into());
    objective.put_string("DisplayName", "Jumps".into());
    objective.put_bool("display_auto_update", true);
    let mut payload = NbtCompound::new();
    payload.put_list("Objectives", vec![NbtTag::Compound(objective)]);
    let mut slots = NbtCompound::new();
    slots.put_string("sidebar.team.dark_red", "custom".into());
    payload.put_compound("DisplaySlots", slots);
    write_scoreboard(folder.path(), payload.clone(), 5023).unwrap();
    let root = read_scoreboard(folder.path()).unwrap().unwrap();
    assert_eq!(root.get_int("DataVersion"), Some(5023));
    assert_eq!(root.get_compound("data"), Some(&payload));
}

#[test]
fn scoreboard_counts_each_container_once_up_to_vanilla_limit() {
    let folder = tempfile::tempdir().unwrap();
    let path = ensure_minecraft_data_dir(folder.path())
        .unwrap()
        .join("scoreboard.dat");
    for lists in [false, true] {
        for containers in [512, 513] {
            // Keep the decoder fixture independent of the writer at the depth boundary.
            let mut id = pumpkin_nbt::STRING_ID;
            let mut nested = vec![0, 4, b'l', b'e', b'a', b'f'];
            // The saved-data root and data compounds account for two containers.
            for _ in 2..containers {
                let mut outer = if lists {
                    vec![id, 0, 0, 0, 1]
                } else {
                    vec![id, 0, 0]
                };
                outer.extend(nested);
                if !lists {
                    outer.push(pumpkin_nbt::END_ID);
                }
                nested = outer;
                id = if lists {
                    pumpkin_nbt::LIST_ID
                } else {
                    pumpkin_nbt::COMPOUND_ID
                };
            }
            let mut bytes = vec![10, 0, 0, 10, 0, 4, b'd', b'a', b't', b'a', id, 0, 6];
            bytes.extend_from_slice(b"nested");
            bytes.extend(nested);
            bytes.extend_from_slice(&[0, 0]);
            let mut encoded = GzEncoder::new(Vec::new(), Compression::default());
            encoded.write_all(&bytes).unwrap();
            fs::write(&path, encoded.finish().unwrap()).unwrap();
            let result = read_scoreboard(folder.path());
            if containers == 512 {
                let root = result.unwrap().unwrap();
                let mut value = root.get_compound("data").unwrap().get("nested").unwrap();
                for _ in 2..containers {
                    value = if lists {
                        &value.extract_list().unwrap()[0]
                    } else {
                        value.extract_compound().unwrap().get("").unwrap()
                    };
                }
                assert_eq!(value.extract_string(), Some("leaf"));
                // Newly accepted documents must survive the real atomic writer too.
                write_scoreboard(
                    folder.path(),
                    root.get_compound("data").unwrap().clone(),
                    5023,
                )
                .unwrap();
                assert!(read_scoreboard(folder.path()).unwrap().is_some());
            } else {
                assert!(
                    result.is_err(),
                    "Accepted {containers} containers, lists={lists}"
                );
            }
        }
    }
}

#[test]
fn scoreboard_corrupt_read_preserves_original_bytes() {
    let folder = tempfile::tempdir().unwrap();
    let path = ensure_minecraft_data_dir(folder.path())
        .unwrap()
        .join("scoreboard.dat");
    let original = b"player data which must never be silently replaced";
    fs::write(&path, original).unwrap();
    assert!(read_scoreboard(folder.path()).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn scoreboard_rejects_missing_compound_terminator_and_trailing_nbt() {
    let folder = tempfile::tempdir().unwrap();
    let path = ensure_minecraft_data_dir(folder.path())
        .unwrap()
        .join("scoreboard.dat");
    let mut root = NbtCompound::new();
    root.put_compound("data", NbtCompound::new());
    let complete = Nbt::new(String::new(), root).write().to_vec();
    let mut truncated = complete.clone();
    truncated.pop();
    let mut trailing = complete;
    trailing.push(0);
    for bytes in [truncated, trailing] {
        let mut encoded = GzEncoder::new(Vec::new(), Compression::default());
        encoded.write_all(&bytes).unwrap();
        let original = encoded.finish().unwrap();
        fs::write(&path, &original).unwrap();
        assert!(read_scoreboard(folder.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn scoreboard_interrupted_atomic_save_keeps_previous_document() {
    let folder = tempfile::tempdir().unwrap();
    let mut payload = NbtCompound::new();
    payload.put_list("Teams", vec![]);
    write_scoreboard(folder.path(), payload.clone(), 5023).unwrap();
    let path = minecraft_data_dir(folder.path()).join("scoreboard.dat");
    let original = fs::read(&path).unwrap();
    payload.put_list("Objectives", vec![]);
    let result = super::super::data_files::interrupt_file("scoreboard.dat", || {
        write_scoreboard(folder.path(), payload, 5023)
    });
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn scoreboard_rejects_oversized_compressed_document() {
    let folder = tempfile::tempdir().unwrap();
    let path = ensure_minecraft_data_dir(folder.path())
        .unwrap()
        .join("scoreboard.dat");
    File::create(&path)
        .unwrap()
        .set_len(MAX_SCOREBOARD_BYTES + 1)
        .unwrap();
    assert!(read_scoreboard(folder.path()).is_err());
    assert_eq!(fs::metadata(path).unwrap().len(), MAX_SCOREBOARD_BYTES + 1);
}

#[test]
fn scoreboard_namespaced_path_wins_and_corruption_never_falls_back() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("data/minecraft")).unwrap();
    let mut legacy = NbtCompound::new();
    legacy.put_compound("data", NbtCompound::new());
    super::super::data_files::write_saved_data(
        &directory.path().join("data/scoreboard.dat"),
        legacy,
    )
    .unwrap();
    let mut data = NbtCompound::new();
    data.put_string("marker", "canonical".into());
    super::write_scoreboard(directory.path(), data.clone(), 5023).unwrap();
    assert_eq!(
        super::read_scoreboard(directory.path())
            .unwrap()
            .unwrap()
            .get_compound("data"),
        Some(&data)
    );
    std::fs::write(
        directory.path().join("data/minecraft/scoreboard.dat"),
        b"corrupt",
    )
    .unwrap();
    assert!(super::read_scoreboard(directory.path()).is_err());
    assert!(directory.path().join("data/scoreboard.dat").is_file());
}
