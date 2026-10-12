use super::*;
use crate::world_info::data_files::{
    interrupt_file, read_wandering_trader, write_wandering_trader,
};

fn interrupted_metadata_save(filename: &'static str) {
    let directory = tempfile::tempdir().unwrap();
    let mut original = LevelData::default(Seed(42));
    original.game_rules.advance_time = false;
    original.rain_time = 120;
    original.raining = true;
    AnvilLevelInfo
        .write_world_info(&original, directory.path())
        .unwrap();
    let mut imported_trader = read_wandering_trader(directory.path());
    imported_trader.spawn_delay = 7123;
    imported_trader.spawn_chance = 31;
    write_wandering_trader(directory.path(), &imported_trader).unwrap();
    let path = minecraft_data_dir(directory.path()).join(filename);
    let before = fs::read(&path).unwrap();
    let mut changed = original;
    changed.game_rules.advance_time = true;
    changed.world_gen_settings.seed = 99;
    changed.rain_time = 240;
    let error = interrupt_file(filename, || {
        AnvilLevelInfo.write_world_info(&changed, directory.path())
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Injected saved-data interruption")
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "Interrupted {filename} replaced its previous document"
    );
    assert!(fs::read_dir(path.parent().unwrap()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")
    }));

    // Restart through the actual level metadata reader, including authoritative data files.
    let restored = AnvilLevelInfo.read_world_info(directory.path()).unwrap();
    match filename {
        "game_rules.dat" => assert!(!restored.game_rules.advance_time),
        "world_gen_settings.dat" => assert_eq!(restored.world_gen_settings.seed, 42),
        "weather.dat" => assert_eq!(restored.rain_time, 120),
        "wandering_trader.dat" => {
            assert_eq!(read_wandering_trader(directory.path()), imported_trader);
        }
        _ => unreachable!(),
    }
    AnvilLevelInfo
        .write_world_info(&changed, directory.path())
        .unwrap();
    let retried = AnvilLevelInfo.read_world_info(directory.path()).unwrap();
    assert!(retried.game_rules.advance_time);
    assert_eq!(retried.world_gen_settings.seed, 99);
    assert_eq!(retried.rain_time, 240);
    assert_eq!(read_wandering_trader(directory.path()), imported_trader);
}

#[test]
fn interrupted_metadata_game_rules_survives_restart() {
    interrupted_metadata_save("game_rules.dat");
}

#[test]
fn interrupted_metadata_world_gen_settings_survives_restart() {
    interrupted_metadata_save("world_gen_settings.dat");
}

#[test]
fn interrupted_metadata_weather_survives_restart() {
    interrupted_metadata_save("weather.dat");
}

#[test]
fn interrupted_metadata_wandering_trader_survives_restart() {
    interrupted_metadata_save("wandering_trader.dat");
}

#[test]
fn late_metadata_game_rules_failure_still_publishes_independent_documents() {
    let directory = tempfile::tempdir().unwrap();
    let mut info = LevelData::default(Seed(42));
    AnvilLevelInfo
        .write_world_info(&info, directory.path())
        .unwrap();
    let failed = minecraft_data_dir(directory.path()).join("game_rules.dat");
    let backup = directory.path().join("game-rules-before-fault.dat");
    let original = fs::read(&failed).unwrap();
    fs::rename(&failed, &backup).unwrap();
    fs::create_dir(&failed).unwrap();
    info.game_rules.advance_time = false;
    info.world_gen_settings.seed = 99;
    info.rain_time = 240;
    let expected = super::super::data_files::DimensionClock {
        total_ticks: 812,
        partial_tick: 0.25,
        rate: 0.75,
        paused: true,
    };
    info.dimension_clocks
        .insert("minecraft:the_end".into(), expected.clone());
    let result = AnvilLevelInfo.write_world_info(&info, directory.path());
    fs::remove_dir(&failed).unwrap();
    fs::rename(&backup, &failed).unwrap();
    assert!(result.is_err());
    assert_eq!(fs::read(&failed).unwrap(), original);
    let saved = AnvilLevelInfo.read_world_info(directory.path()).unwrap();
    assert_eq!(
        super::super::data_files::read_world_clocks(directory.path())
            .clocks
            .get("minecraft:the_end"),
        Some(&expected),
        "A game-rules failure skipped independently writable clock data"
    );
    assert_eq!(saved.world_gen_settings.seed, 99);
    assert_eq!(saved.rain_time, 240);
    AnvilLevelInfo
        .write_world_info(&info, directory.path())
        .unwrap();
    assert!(
        !AnvilLevelInfo
            .read_world_info(directory.path())
            .unwrap()
            .game_rules
            .advance_time
    );
}
