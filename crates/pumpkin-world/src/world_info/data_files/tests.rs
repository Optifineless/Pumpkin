use super::*;
use pumpkin_nbt::nbt_compress::write_gzip_compound_tag;

#[test]
fn legacy_integer_clock_ticks_load_save_as_longs() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = ensure_minecraft_data_dir(directory.path()).unwrap();
    let path = data_dir.join("world_clocks.dat");
    let mut overworld = NbtCompound::new();
    overworld.put_int("total_ticks", 5000);
    let mut nether = NbtCompound::new();
    nether.put_int("total_ticks", 0);
    let mut custom = NbtCompound::new();
    custom.put_long("total_ticks", 8_000_000_000);
    custom.put_float("partial_tick", 0.25);
    custom.put_float("rate", 0.5);
    custom.put_bool("paused", true);
    let mut data = NbtCompound::new();
    data.put_compound("minecraft:overworld", overworld);
    data.put_compound("minecraft:the_nether", nether);
    data.put_compound("test:clock", custom);
    data.put_int(
        "DataVersion",
        crate::world_info::MAXIMUM_SUPPORTED_WORLD_DATA_VERSION,
    );
    let mut root = NbtCompound::new();
    root.put_compound("data", data);
    write_gzip_compound_tag(root, File::create(&path).unwrap()).unwrap();

    let clocks = read_world_clocks(directory.path());
    assert_eq!(clocks.clocks["minecraft:overworld"].total_ticks, 5000);
    assert_eq!(clocks.clocks["minecraft:overworld"].partial_tick, 0.0);
    assert_eq!(clocks.clocks["minecraft:overworld"].rate, 1.0);
    assert!(!clocks.clocks["minecraft:overworld"].paused);
    assert_eq!(clocks.clocks["minecraft:the_nether"].total_ticks, 0);
    assert_eq!(clocks.clocks["test:clock"].total_ticks, 8_000_000_000);
    assert_eq!(clocks.clocks["test:clock"].partial_tick, 0.25);
    assert_eq!(clocks.clocks["test:clock"].rate, 0.5);
    assert!(clocks.clocks["test:clock"].paused);
    write_world_clocks(directory.path(), &clocks).unwrap();

    let saved = read_gzip_compound_tag(File::open(path).unwrap()).unwrap();
    let data = saved.get_compound("data").unwrap();
    for (key, ticks) in [
        ("minecraft:overworld", 5000),
        ("minecraft:the_nether", 0),
        ("test:clock", 8_000_000_000),
    ] {
        assert_eq!(
            data.get_compound(key).unwrap().get_long("total_ticks"),
            Some(ticks)
        );
    }
    assert_eq!(read_world_clocks(directory.path()), clocks);
}

#[test]
fn malformed_clock_fractions_default_while_valid_boundaries_survive() {
    let directory = tempfile::tempdir().unwrap();
    let path = ensure_minecraft_data_dir(directory.path())
        .unwrap()
        .join("world_clocks.dat");
    let mut data = NbtCompound::new();
    let invalid = [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -0.25,
        1.0,
        f32::MAX,
    ];
    let valid = [0.0, 0.25, f32::from_bits(1.0f32.to_bits() - 1)];
    for (index, fraction) in invalid.into_iter().chain(valid).enumerate() {
        let mut clock = NbtCompound::new();
        clock.put_long("total_ticks", 5000);
        clock.put_float("partial_tick", fraction);
        clock.put_float("rate", 0.5);
        clock.put_bool("paused", true);
        data.put_compound(&format!("test:{index}"), clock);
    }
    let mut root = NbtCompound::new();
    root.put_compound("data", data);
    write_gzip_compound_tag(root, File::create(path).unwrap()).unwrap();

    let clocks = read_world_clocks(directory.path());
    for (index, expected) in [0.0; 6].into_iter().chain(valid).enumerate() {
        let clock = &clocks.clocks[&format!("test:{index}")];
        assert_eq!(clock.partial_tick, expected, "clock {index}");
        assert_eq!(clock.total_ticks, 5000);
        assert_eq!(clock.rate, 0.5);
        assert!(clock.paused);
    }
    write_world_clocks(directory.path(), &clocks).unwrap();
    assert_eq!(read_world_clocks(directory.path()), clocks);
}
