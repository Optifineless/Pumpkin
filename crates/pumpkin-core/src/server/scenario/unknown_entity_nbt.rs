//! Issue #414 regressions through disk activation, real watcher retirement and player rejoin.
use super::*;
use pumpkin_nbt::{
    compound::NbtCompound,
    serializer::{NbtWriteHelper, NbtWriteHelperJava},
    tag::NbtTag,
};
use pumpkin_world::{
    chunk::{ChunkEntityData, io::DirtyFlag},
    data::player_data::PlayerDataStorage,
};
use std::sync::atomic::{AtomicBool, Ordering};

const FUTURE: &str = "murgicraft:future";
const NUMBER: &str = "murgicraft:future_number";
const CHUNK: Vector2<i32> = Vector2::new(0, 0);

fn record(kind: &str, id: Uuid) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", format!("minecraft:{kind}"));
    nbt.put_uuid("UUID", id);
    nbt.put_list("Pos", [10.5, 64.0, 10.5].map(NbtTag::Double).to_vec());
    nbt.put_bool("NoAI", true);
    nbt.put_bool("PersistenceRequired", true);
    nbt.put_bool("Invulnerable", true);
    if kind == "item" {
        nbt.put_short("Health", 5);
        nbt.put_short("PickupDelay", 100);
        let mut item = NbtCompound::new();
        item.put_string("id", "minecraft:diamond".into());
        item.put_int("count", 7);
        item.put_long("murgicraft:item_future", 914);
        nbt.put_compound("Item", item);
    } else {
        nbt.put_float("Health", 17.0);
    }
    let mut future = NbtCompound::new();
    future.put_int("a", 1);
    nbt.put_compound(FUTURE, future);
    nbt.put_long(NUMBER, 0x0102_0304_0506_0708);
    nbt
}

async fn seed(scenario: &Scenario, records: Vec<NbtCompound>) {
    let world = scenario.world(&Dimension::OVERWORLD);
    let chunk = Arc::new(ChunkEntityData {
        x: 0,
        z: 0,
        data: pumpkin_util::sync::Mutex::new(records),
        live: AtomicBool::new(false),
        dirty: DirtyFlag::new(true),
    });
    scenario
        .drive(
            "unknown entity fixture save",
            world.level.write_entity_chunks(vec![(CHUNK, chunk)]),
        )
        .await;
    scenario
        .drive(
            "unknown entity fixture flush",
            world.level.drain_entity_storage(),
        )
        .await
        .unwrap();
    assert!(world.entities.load().is_empty());
}

fn encoded(tag: &NbtTag) -> Vec<u8> {
    // These fixtures contain a single-entry compound, so HashMap ordering cannot affect bytes.
    let mut bytes = Vec::new();
    let mut writer = NbtWriteHelperJava::new(&mut bytes);
    writer.write_u8(tag.get_type_id()).unwrap();
    tag.clone().serialize_data(&mut writer).unwrap();
    bytes
}

fn assert_tags(saved: &NbtCompound, original: &NbtCompound, keys: &[&str]) {
    for key in keys {
        let tag = saved
            .get(key)
            .unwrap_or_else(|| panic!("lost retained tag {key}"));
        assert_eq!(
            encoded(tag),
            encoded(original.get(key).unwrap()),
            "changed {key}"
        );
    }
}

fn find(records: &[NbtCompound], id: Uuid) -> &NbtCompound {
    records
        .iter()
        .find(|nbt| nbt.get_uuid("UUID") == Some(id))
        .unwrap()
}

async fn travel(scenario: &Scenario, session: &Session, away: bool) {
    let target = if away { "163.5 64 3.5" } else { "3.5 64 3.5" };
    scenario.command(&format!("tp {} {target}", session.player.gameprofile.name));
    session.loaded(&scenario.server);
    scenario
        .until("unknown entity watcher tasks", || {
            scenario.server.tasks.is_empty() && session.is_idle()
        })
        .await;
    scenario.ticks(1).await;
}

async fn unload(scenario: &Scenario, session: &Session) -> Vec<NbtCompound> {
    let world = session.player.world();
    scenario.prepare_area(&world, Vector2::new(10, 0)).await;
    travel(scenario, session, true).await;
    assert!(
        world.entities.load().is_empty(),
        "real watcher unload did not retire entities"
    );
    assert!(world.level.get_entity_chunk_sync(&CHUNK).is_none());
    let chunk = scenario
        .drive(
            "unknown entity saved chunk",
            world.level.get_entity_chunk(CHUNK),
        )
        .await
        .unwrap();
    assert!(!chunk.live.load(Ordering::Acquire));
    chunk.data.lock().unwrap().clone()
}

fn snapshot(entity: &dyn EntityBase) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    nbt
}

async fn root_roundtrip(kind: &str) {
    let scenario = Scenario::new().await;
    let id = Uuid::new_v4();
    let original = record(kind, id);
    seed(&scenario, vec![original.clone()]).await;
    let session = scenario.join(Uuid::new_v4(), "FutureRoot").await;
    let world = session.player.world();
    let entity = world.get_entity_by_uuid(id).unwrap();
    let before = snapshot(entity.as_ref());
    scenario.ticks(3).await;
    let saved = unload(&scenario, &session).await;
    let saved = find(&saved, id);
    assert_tags(saved, &original, &[FUTURE, NUMBER]);
    if kind == "item" {
        assert_eq!(
            saved
                .get_compound("Item")
                .unwrap()
                .get_long("murgicraft:item_future"),
            Some(914)
        );
    }
    for key in [
        "id",
        "UUID",
        "Health",
        "Item",
        "NoAI",
        "PersistenceRequired",
    ] {
        assert_eq!(saved.get(key), before.get(key), "known field {key} changed");
    }
    travel(&scenario, &session, false).await;
    let loaded = world.get_entity_by_uuid(id).unwrap();
    let reloaded = snapshot(loaded.as_ref());
    assert_tags(&reloaded, &original, &[FUTURE, NUMBER]);
    if kind == "item" {
        assert_eq!(
            reloaded
                .get_compound("Item")
                .unwrap()
                .get_long("murgicraft:item_future"),
            Some(914)
        );
        let stack = loaded
            .get_item_entity()
            .unwrap()
            .get_item_stack()
            .lock()
            .unwrap()
            .clone();
        assert_eq!(stack.item.registry_key, "diamond");
        assert_eq!(stack.item_count, 7);
    }
    scenario.stop().await;
}

#[tokio::test]
async fn unknown_entity_nbt_villager_chunk_roundtrip() {
    run(root_roundtrip("villager")).await;
}

#[tokio::test]
async fn unknown_entity_nbt_horse_chunk_roundtrip() {
    run(root_roundtrip("horse")).await;
}

#[tokio::test]
async fn unknown_entity_nbt_zombie_chunk_roundtrip() {
    run(root_roundtrip("zombie")).await;
}

#[tokio::test]
async fn unknown_entity_nbt_item_chunk_roundtrip() {
    run(root_roundtrip("item")).await;
}

#[tokio::test]
async fn unknown_entity_nbt_villager_rollback_age_fields() {
    run(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut original = record("villager", id);
        original.put_int("Age", -24_000);
        original.put_int("ForcedAge", 17);
        original.put_byte("AgeLocked", 1);
        original.put_int("murgicraft:VillagerAgeSchema", 1);
        seed(&scenario, vec![original.clone()]).await;
        let session = scenario.join(Uuid::new_v4(), "RollbackAge").await;
        scenario.ticks(3).await;
        let keys = [
            "Age",
            "ForcedAge",
            "AgeLocked",
            "murgicraft:VillagerAgeSchema",
        ];
        let saved = unload(&scenario, &session).await;
        assert_tags(find(&saved, id), &original, &keys);
        travel(&scenario, &session, false).await;
        let entity = session.player.world().get_entity_by_uuid(id).unwrap();
        assert_tags(&snapshot(entity.as_ref()), &original, &keys);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn unknown_entity_nbt_live_owned_tags_and_omissions_win() {
    run(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut original = record("villager", id);
        original.put_bool("HasVisualFire", true);
        seed(&scenario, vec![original.clone()]).await;
        let session = scenario.join(Uuid::new_v4(), "OwnedTags").await;
        let entity = session.player.world().get_entity_by_uuid(id).unwrap();
        entity.get_living_entity().unwrap().set_health(7.0);
        let mut base_only = NbtCompound::new();
        base_only.put_float("Health", 1.0);
        entity.get_entity().read_nbt_non_mut(&base_only);
        assert_eq!(entity.get_living_entity().unwrap().health.load(), 7.0);
        entity
            .get_entity()
            .has_visual_fire
            .store(false, Ordering::Relaxed);
        entity.get_entity().set_pos(Vector3::new(11.5, 64.0, 9.5));
        scenario.ticks(2).await;
        let position = entity.get_entity().pos.load();
        let expected_position = NbtTag::List(
            [position.x, position.y, position.z]
                .map(NbtTag::Double)
                .to_vec(),
        );
        let saved = unload(&scenario, &session).await;
        let saved = find(&saved, id);
        assert_eq!(saved.get_float("Health"), Some(7.0));
        assert_eq!(saved.get("Pos"), Some(&expected_position));
        assert!(
            saved.get("HasVisualFire").is_none(),
            "stale omitted known tag resurrected"
        );
        assert_tags(saved, &original, &[FUTURE, NUMBER]);
        travel(&scenario, &session, false).await;
        let entity = session.player.world().get_entity_by_uuid(id).unwrap();
        assert_eq!(entity.get_living_entity().unwrap().health.load(), 7.0);
        assert!(!entity.get_entity().has_visual_fire.load(Ordering::Relaxed));
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn unknown_entity_nbt_passengers_and_leashed_entity() {
    run(async {
        let scenario = Scenario::new().await;
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut mount = record("horse", ids[0]);
        let passenger = record("zombie", ids[1]);
        let leashed = record("cow", ids[2]);
        mount.put_list("Passengers", vec![NbtTag::Compound(passenger.clone())]);
        seed(&scenario, vec![mount.clone(), leashed.clone()]).await;
        let session = scenario.join(Uuid::new_v4(), "LinkedTags").await;
        let world = session.player.world();
        let horse = world.get_entity_by_uuid(ids[0]).unwrap();
        let rider = world.get_entity_by_uuid(ids[1]).unwrap();
        let cow = world.get_entity_by_uuid(ids[2]).unwrap();
        assert!(rider.get_entity().has_vehicle());
        cow.get_entity().leash_to(horse.clone());
        assert!(cow.get_entity().is_leashed());
        scenario.ticks(2).await;
        let saved = unload(&scenario, &session).await;
        let saved_mount = find(&saved, ids[0]);
        let saved_rider = saved_mount.get_list("Passengers").unwrap()[0]
            .extract_compound()
            .unwrap();
        assert_tags(saved_rider, &passenger, &[FUTURE, NUMBER]);
        assert_tags(find(&saved, ids[2]), &leashed, &[FUTURE, NUMBER]);
        assert_tags(saved_mount, &mount, &[FUTURE, NUMBER]);
        travel(&scenario, &session, false).await;
        for (id, original) in ids.into_iter().zip([mount, passenger, leashed]) {
            let entity = world.get_entity_by_uuid(id).unwrap();
            assert_tags(&snapshot(entity.as_ref()), &original, &[FUTURE, NUMBER]);
        }
        assert!(
            world
                .get_entity_by_uuid(ids[1])
                .unwrap()
                .get_entity()
                .has_vehicle()
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn unknown_entity_nbt_dismount_does_not_restore_stale_passengers() {
    run(async {
        let scenario = Scenario::new().await;
        let mount_id = Uuid::new_v4();
        let rider_id = Uuid::new_v4();
        let mut mount = record("horse", mount_id);
        mount.put_list(
            "Passengers",
            vec![NbtTag::Compound(record("zombie", rider_id))],
        );
        seed(&scenario, vec![mount.clone()]).await;
        let session = scenario.join(Uuid::new_v4(), "DismountTags").await;
        let world = session.player.world();
        let horse = world.get_entity_by_uuid(mount_id).unwrap();
        let rider = world.get_entity_by_uuid(rider_id).unwrap();
        horse
            .get_entity()
            .remove_passenger_sync(rider.get_entity().entity_id);
        assert!(!rider.get_entity().has_vehicle());
        let saved = unload(&scenario, &session).await;
        assert!(find(&saved, mount_id).get("Passengers").is_none());
        assert!(find(&saved, rider_id).get("Passengers").is_none());
        assert_tags(find(&saved, mount_id), &mount, &[FUTURE, NUMBER]);
        travel(&scenario, &session, false).await;
        assert!(
            !world
                .get_entity_by_uuid(rider_id)
                .unwrap()
                .get_entity()
                .has_vehicle()
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn unknown_entity_nbt_merge_and_native_nbt_keep_nested_unknown_fields() {
    run(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut original = record("villager", id);
        let mut data = NbtCompound::new();
        data.put_string("type", "minecraft:taiga".into());
        data.put_string("profession", "minecraft:librarian".into());
        data.put_int("level", 3);
        data.put_long(NUMBER, 913);
        original.put_compound("VillagerData", data);
        seed(&scenario, vec![original.clone()]).await;
        let session = scenario.join(Uuid::new_v4(), "MergedTags").await;
        let entity = session.player.world().get_entity_by_uuid(id).unwrap();
        assert_eq!(
            scenario.command(&format!(
                "data merge entity {id} {{Health:9f,\"murgicraft:merged\":{{a:2}}}}"
            )),
            1
        );
        assert_eq!(entity.get_living_entity().unwrap().health.load(), 9.0);
        // Native plugins use this public EntityBase NBT boundary.
        let mut patch = NbtCompound::new();
        patch.put_float("Health", 6.0);
        patch.put_long("murgicraft:native", 721);
        entity.read_nbt_non_mut(&patch);
        assert_eq!(entity.get_living_entity().unwrap().health.load(), 6.0);
        let mut data = snapshot(entity.as_ref())
            .get_compound("VillagerData")
            .unwrap()
            .clone();
        data.put_int("level", 4);
        let mut patch = NbtCompound::new();
        patch.put_float("Health", 6.0);
        patch.put_compound("VillagerData", data);
        entity.read_nbt_non_mut(&patch);
        let saved = unload(&scenario, &session).await;
        let saved = find(&saved, id);
        assert_eq!(saved.get_float("Health"), Some(6.0));
        let data = saved.get_compound("VillagerData").unwrap();
        assert_eq!(data.get_int("level"), Some(4));
        assert_eq!(
            data.get_long(NUMBER),
            Some(913),
            "lost nested entity-type tag"
        );
        assert_eq!(saved.get_long("murgicraft:native"), Some(721));
        assert_eq!(
            saved
                .get_compound("murgicraft:merged")
                .unwrap()
                .get_int("a"),
            Some(2)
        );
        assert_tags(saved, &original, &[FUTURE, NUMBER]);
        travel(&scenario, &session, false).await;
        let reloaded = snapshot(
            session
                .player
                .world()
                .get_entity_by_uuid(id)
                .unwrap()
                .as_ref(),
        );
        assert_eq!(
            reloaded
                .get_compound("VillagerData")
                .unwrap()
                .get_long(NUMBER),
            Some(913)
        );
        assert_eq!(
            scenario.command(&format!("data remove entity {id} \"murgicraft:merged\"")),
            1
        );
        let saved = unload(&scenario, &session).await;
        assert!(find(&saved, id).get("murgicraft:merged").is_none());
        assert_eq!(find(&saved, id).get_long("murgicraft:native"), Some(721));
        travel(&scenario, &session, false).await;
        let entity = session.player.world().get_entity_by_uuid(id).unwrap();
        assert!(snapshot(entity.as_ref()).get("murgicraft:merged").is_none());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn unknown_entity_nbt_brain_future_memory_survives_chunk_roundtrip() {
    run(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut original = record("villager", id);
        let mut value = NbtCompound::new();
        value.put_long("value", 722);
        let mut memories = NbtCompound::new();
        memories.put_compound("murgicraft:future_memory", value.clone());
        let mut brain = NbtCompound::new();
        brain.put_compound("memories", memories);
        original.put_compound("Brain", brain);
        seed(&scenario, vec![original]).await;
        let session = scenario.join(Uuid::new_v4(), "FutureBrain").await;
        scenario.ticks(3).await;
        let saved = unload(&scenario, &session).await;
        let memory = find(&saved, id)
            .get_compound("Brain")
            .and_then(|brain| brain.get_compound("memories"))
            .and_then(|memories| memories.get_compound("murgicraft:future_memory"));
        assert_eq!(memory, Some(&value), "lost unknown brain memory");
        travel(&scenario, &session, false).await;
        let saved = snapshot(
            session
                .player
                .world()
                .get_entity_by_uuid(id)
                .unwrap()
                .as_ref(),
        );
        assert_eq!(
            saved
                .get_compound("Brain")
                .unwrap()
                .get_compound("memories")
                .unwrap()
                .get_compound("murgicraft:future_memory"),
            Some(&value)
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn unknown_entity_nbt_player_disconnect_rejoin() {
    run(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut original = record("player", id);
        original.put_bool("Invulnerable", false);
        original.put_int("playerGameType", 0);
        original.put_string("Dimension", Dimension::OVERWORLD.minecraft_name.into());
        let storage = PlayerDataStorage::new(scenario.directory.path().join("players"), true);
        storage
            .save_snapshot(storage.snapshot(&id, || original.clone()))
            .unwrap();
        let session = scenario.join(id, "PlayerTags").await;
        scenario.ticks(2).await;
        session.player.living_entity.set_health(11.0);
        scenario.leave(&session).await;
        let (_, saved) = storage.load_player_data(&id).unwrap();
        assert_eq!(saved.get_float("Health"), Some(11.0));
        assert_tags(&saved, &original, &[FUTURE, NUMBER]);
        let rejoined = scenario.join(id, "PlayerTags").await;
        assert_eq!(rejoined.player.living_entity.health.load(), 11.0);
        let mut nbt = NbtCompound::new();
        crate::entity::NBTStorage::write_nbt(rejoined.player.as_ref(), &mut nbt);
        assert_tags(&nbt, &original, &[FUTURE, NUMBER]);
        scenario.leave(&rejoined).await;
        let (_, saved) = storage.load_player_data(&id).unwrap();
        assert_tags(&saved, &original, &[FUTURE, NUMBER]);
        scenario.stop().await;
    })
    .await;
}
