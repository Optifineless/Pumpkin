use super::*;
#[test]
fn vanilla_saved_words_defaults_and_restart_continue_the_stream() {
    use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
    // Vanilla CODECs: the source is a two-long array, not the initial world seed.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data/minecraft")).unwrap();
    let mut sequence = NbtCompound::new();
    sequence.put("source", NbtTag::LongArray(vec![1, 2]));
    let mut entries = NbtCompound::new();
    entries.put_compound("minecraft:test", sequence);
    let mut data = NbtCompound::new();
    data.put_int("salt", 42);
    data.put_bool("include_world_seed", false);
    data.put_bool("include_sequence_id", false);
    data.put_compound("sequences", entries);
    let mut root = NbtCompound::new();
    root.put_compound("data", data);
    root.put_int("DataVersion", 0);
    std::fs::write(
        dir.path().join("data/minecraft/random_sequences.dat"),
        Nbt::new(String::new(), root).write(),
    )
    .unwrap();
    let id = Identifier::parse("minecraft:test").unwrap();
    let mut manager = RandomSequences::load(dir.path()).unwrap();
    assert_eq!(manager.salt, 42);
    assert!(!manager.include_world_seed && !manager.include_sequence_id);
    assert!(!manager.dirty);
    assert_eq!(manager.get_or_create(&id, 999).random().next_i64(), 393217);
    assert!(manager.dirty);
    manager.save(dir.path(), 0).unwrap();
    assert!(!manager.dirty);
    let mut restored = RandomSequences::load(dir.path()).unwrap();
    assert_eq!(
        restored.get_or_create(&id, 999).random().next_i64(),
        669327710093319
    );
    manager.set_seed_defaults(-7, false, true);
    manager.reset_with_options(&id, 45, 99, false, false);
    manager.save(dir.path(), 0).unwrap();
    let mut restored = RandomSequences::load(dir.path()).unwrap();
    assert_eq!(restored.salt, -7);
    assert!(!restored.include_world_seed && restored.include_sequence_id);
    assert_eq!(
        restored.get_or_create(&id, 0).random().next_i64(),
        manager.get_or_create(&id, 0).random().next_i64()
    );
    restored.clear();
    restored.save(dir.path(), 0).unwrap();
    assert!(
        RandomSequences::load(dir.path())
            .unwrap()
            .sequences
            .is_empty()
    );
}
