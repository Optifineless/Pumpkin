use super::*;
use pumpkin_data::tracked_data;
use pumpkin_protocol::java::client::play::Metadata;

#[derive(Clone, Copy, Debug)]
pub(super) struct Initial {
    pub(super) age: i32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) eye: f64,
}

// Stage login and watcher completion before simulation. Reusing the watched-storage
// activation entry leaves the queued lifecycle activation idempotent.
pub(super) async fn activate(
    scenario: &Scenario,
    name: &str,
) -> (Rc<Session>, std::collections::BTreeMap<Uuid, Initial>) {
    let profile = GameProfile {
        id: Uuid::new_v4(),
        name: name.into(),
        properties: arc_swap::ArcSwap::from_pointee(Vec::new()),
        profile_actions: None,
    };
    let (client, incoming) = captured_client(profile.clone());
    let (player, world) = scenario
        .drive(
            "age staged login",
            scenario.server.add_player(
                Arc::new(ClientPlatform::Java(client)),
                profile,
                Some(PlayerConfig {
                    view_distance: std::num::NonZeroU8::new(2).unwrap(),
                    ..PlayerConfig::default()
                }),
            ),
        )
        .await
        .unwrap();
    let session = Rc::new(Session {
        player,
        incoming: RefCell::new(incoming),
        captured: RefCell::new(Vec::new()),
    });
    session.client().set_player(session.player.clone());
    scenario.sessions.borrow_mut().push(session.clone());
    scenario
        .drive(
            "age staged join packets",
            world.spawn_java_player(
                &scenario.server.basic_config,
                &session.player,
                &scenario.server,
            ),
        )
        .await;
    session.loaded(&scenario.server);
    scenario
        .until("age watcher/storage completion", || {
            session.is_idle()
                && world.level.is_chunk_watched(&Vector2::new(0, 0))
                && world
                    .level
                    .get_entity_chunk_sync(&Vector2::new(0, 0))
                    .is_some()
        })
        .await;
    let storage = world
        .level
        .get_entity_chunk_sync(&Vector2::new(0, 0))
        .unwrap();
    assert!(world.make_chunk_entities_live(&storage, Some(&session.player)));
    let initial = world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            let villager = entity.cast_any().downcast_ref::<VillagerEntity>()?;
            let base = villager.get_entity();
            base.age.store(200, Relaxed);
            Some((
                base.entity_uuid,
                Initial {
                    age: villager.biological_age().age,
                    width: base.width(),
                    height: base.height(),
                    eye: base.get_eye_height(),
                },
            ))
        })
        .collect();
    (session, initial)
}

// Metadata is searched only inside the real entity-data packet for this runtime ID.
pub(super) fn baby_updates(packets: &[Bytes], id: i32, baby: bool) -> usize {
    let mut expected = Vec::new();
    Metadata::new(tracked_data::villager::DATA_BABY_ID, baby)
        .write(&mut expected, &CURRENT_MC_VERSION)
        .unwrap();
    packets
        .iter()
        .filter(|packet| {
            let mut input = packet.as_ref();
            input.get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SET_ENTITY_DATA.0
                && input.get_var_int().unwrap().0 == id
                && input.windows(expected.len()).any(|bytes| bytes == expected)
        })
        .count()
}

pub(super) fn bedrock_ages(packets: &[Bytes], runtime: i32, spawn: bool) -> Vec<(bool, f32, f32)> {
    use pumpkin_protocol::bedrock::client::set_actor_data::{entity_data_flag, entity_data_key};
    use pumpkin_protocol::{
        codec::{var_long::VarLong, var_uint::VarUInt, var_ulong::VarULong},
        serial::PacketRead,
    };
    packets
        .iter()
        .filter_map(|packet| {
            let mut input = packet.as_ref();
            assert_eq!(
                u8::read(&mut input).unwrap(),
                pumpkin_protocol::bedrock::BEDROCK_GAME_PACKET
            );
            let length = VarUInt::read(&mut input).unwrap().0 as usize;
            assert_eq!(length, input.len());
            if VarUInt::read(&mut input).unwrap().0 & 0x3ff != if spawn { 13 } else { 39 } {
                return None;
            }
            if spawn {
                VarLong::read(&mut input).unwrap();
            }
            if VarULong::read(&mut input).unwrap().0 != runtime as u64 {
                return None;
            }
            if spawn {
                assert_eq!(String::read(&mut input).unwrap(), "minecraft:villager_v2");
                for _ in 0..10 {
                    f32::read(&mut input).unwrap();
                }
                assert_eq!(
                    VarUInt::read(&mut input).unwrap().0,
                    0,
                    "fixture has no Bedrock attributes"
                );
            }
            let count = VarUInt::read(&mut input).unwrap().0;
            let (mut flags, mut width, mut height) = (None, None, None);
            for _ in 0..count {
                let key = VarUInt::read(&mut input).unwrap().0;
                let kind = VarUInt::read(&mut input).unwrap().0;
                assert_eq!(u8::read(&mut input).unwrap() as u32, kind);
                match kind {
                    0 => {
                        i8::read(&mut input).unwrap();
                    }
                    1 => {
                        i16::read(&mut input).unwrap();
                    }
                    2 => {
                        VarInt::read(&mut input).unwrap();
                    }
                    3 => {
                        let value = f32::read(&mut input).unwrap();
                        if key == entity_data_key::WIDTH {
                            width = Some(value);
                        }
                        if key == entity_data_key::HEIGHT {
                            height = Some(value);
                        }
                    }
                    4 => {
                        String::read(&mut input).unwrap();
                    }
                    7 => {
                        let value = VarLong::read(&mut input).unwrap().0;
                        if key == entity_data_key::FLAGS {
                            flags = Some(value);
                        }
                    }
                    _ => panic!("unexpected villager actor metadata kind {kind}"),
                }
            }
            Some((
                flags? & (1i64 << entity_data_flag::BABY) != 0,
                width?,
                height?,
            ))
        })
        .collect()
}

pub(super) async fn bedrock(scenario: &Scenario) -> TestBedrockPlayer {
    let world = scenario.world(&Dimension::OVERWORLD);
    let observer = TestBedrockPlayer::new(&world).await;
    observer.player.get_entity().set_pos(SPAWN);
    observer.player.watched_section.store(
        pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
            Vector2::new(0, 0),
            std::num::NonZeroU8::new(2).unwrap(),
        ),
    );
    observer
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(Vector2::new(0, 0));
    observer
}
pub(super) async fn restart_age_image(
    scenario: &Scenario,
    ids: &[Uuid; 8],
    records: &[NbtCompound],
    cooldown: i32,
) -> Vec<[Option<NbtTag>; 4]> {
    let mut image = Vec::new();
    for (index, id) in ids.iter().copied().enumerate() {
        let saved = saved_record(scenario, id).await;
        if index >= 6 {
            for key in ["Age", "ForcedAge", "AgeLocked", SCHEMA] {
                assert_eq!(
                    saved.get(key),
                    records[index].get(key),
                    "quarantined exact tag {key}"
                );
            }
        } else {
            assert_eq!(
                saved.get_int("Age"),
                Some([0, 0, -3, cooldown, 0, -4][index]),
                "no offline aging"
            );
            assert_eq!(
                saved.get("ForcedAge"),
                Some(&NbtTag::Int(if index == 4 { 0 } else { 17 }))
            );
            assert!(matches!(saved.get("AgeLocked"), Some(NbtTag::Byte(_))));
            assert_eq!(saved.get_int(SCHEMA), Some(1));
        }
        if index == 5 {
            assert_eq!(
                saved.get_compound("VillagerData"),
                records[index].get_compound("VillagerData"),
                "configured non-plains baby data"
            );
            assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
        } else if index != 2 {
            assert_vanilla_encoding(&saved);
        } else {
            assert_eq!(
                saved
                    .get_compound("VillagerData")
                    .unwrap()
                    .get_string("type"),
                Some("future:alpine"),
                "present-data finalization retained unknown type"
            );
            assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
        }
        for key in [
            "Offers",
            "Inventory",
            "Gossips",
            "Brain",
            "HomeX",
            "HomeY",
            "HomeZ",
        ] {
            assert_eq!(
                saved.get(key),
                records[index].get(key),
                "restart retained {key}"
            );
        }
        image.push(["Age", "ForcedAge", "AgeLocked", SCHEMA].map(|k| saved.get(k).cloned()));
    }
    image
}

pub(super) struct CancelEgg(pub(super) Arc<AtomicBool>);
impl EventHandler<EntitySpawnEvent> for CancelEgg {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if self.0.load(Relaxed) {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn villager_s6_unrelated_elapsed_pose_and_egg_payment_controls() {
    bounded(async {
        let scenario = Scenario::new().await;
        let java = scenario.join(Uuid::new_v4(), "AgeControls").await;
        let world = java.player.world();
        let iron = Scenario::mob(
            &world,
            &EntityType::IRON_GOLEM,
            Vector3::new(6.5, 64.0, 6.5),
        );
        iron.get_mob().unwrap().get_mob_entity().set_no_ai(true);
        iron.get_entity().age.store(400, Relaxed);
        scenario.ticks(2).await;
        assert_eq!(
            iron.get_entity().age.load(Relaxed),
            402,
            "unrelated elapsed cadence"
        );
        let cow = Scenario::mob(&world, &EntityType::COW, Vector3::new(4.0, 64.0, 4.0));
        cow.get_entity().set_pose(EntityPose::Sleeping);
        cow.get_entity().set_pose(EntityPose::Standing);
        assert_eq!(
            (cow.get_entity().width(), cow.get_entity().height()),
            (0.6, 1.8)
        );
        java.player
            .inventory
            .set_stack(0, ItemStack::new(2, &Item::COW_SPAWN_EGG));
        interact(&scenario, &java, cow.get_entity().entity_id);
        assert_eq!(
            java.player.inventory.get_stack(0).item_count,
            1,
            "unrelated egg paid once"
        );
        scenario
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(
                Arc::new(CancelEgg(Arc::new(AtomicBool::new(true)))),
                EventPriority::Normal,
                true,
            );
        interact(&scenario, &java, cow.get_entity().entity_id);
        assert_eq!(
            java.player.inventory.get_stack(0).item_count,
            1,
            "unrelated cancelled egg conserved"
        );
        assert_eq!(
            world
                .entities
                .load()
                .iter()
                .filter(|e| e.get_entity().entity_type == &EntityType::COW)
                .count(),
            2
        );
        scenario.stop().await;
    })
    .await;
}

pub(super) fn age_records(ids: &[Uuid; 8]) -> Vec<NbtCompound> {
    let mut records = vec![
        fixture(ids[0], -1, false),
        fixture(ids[1], 1, false),
        fixture(ids[2], -3, true),
        fixture(ids[3], 200, true),
        villager_fixture(ids[4], true),
        fixture(ids[5], -4, true),
        fixture(ids[6], -7, false),
        fixture(ids[7], -8, false),
    ];
    records[5].put_int(SCHEMA, 1);
    let mut child_data = NbtCompound::new();
    child_data.put_string("type", "minecraft:snow".into());
    child_data.put_string("profession", "minecraft:none".into());
    child_data.put_int("level", 1);
    records[5].put_compound("VillagerData", child_data);
    let mut inventory = records[5].get_list("Inventory").unwrap().to_vec();
    if let NbtTag::Compound(stack) = &mut inventory[0] {
        let mut data = NbtCompound::new();
        data.put_int("age_control", 7);
        let mut components = NbtCompound::new();
        components.put_compound("minecraft:custom_data", data);
        stack.put_compound("components", components);
    }
    records[5].put_list("Inventory", inventory);
    records[6].put_string("Age", "broken-age".into());
    records[6].put_long("ForcedAge", 9);
    records[7].put_int(SCHEMA, 99);
    let mut future_data = records[2].get_compound("VillagerData").unwrap().clone();
    future_data.put_string("type", "future:alpine".into());
    records[2].put_compound("VillagerData", future_data);
    records[2].put_bool("VillagerDataFinalized", false);
    records
}

pub(super) fn real_egg(scenario: &Scenario, java: &Session, parent: &Arc<dyn EntityBase>) -> Uuid {
    let world = java.player.world();
    let id = parent.get_entity().entity_uuid;
    java.player
        .inventory
        .set_stack(0, ItemStack::new(2, &Item::VILLAGER_SPAWN_EGG));
    let cancel = Arc::new(AtomicBool::new(false));
    scenario
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(
            Arc::new(CancelEgg(cancel.clone())),
            EventPriority::Normal,
            true,
        );
    interact(scenario, java, parent.get_entity().entity_id);
    let children: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter(|e| e.cast_any().is::<VillagerEntity>() && e.get_entity().entity_uuid != id)
        .cloned()
        .collect();
    assert_eq!(children.len(), 1, "one admitted egg child");
    assert_eq!(
        java.player.inventory.get_stack(0).item_count,
        1,
        "one egg payment"
    );

    let child_id = children[0].get_entity().entity_uuid;
    assert_eq!(age(scenario, child_id), -24000, "egg biological baby age");
    let child_runtime = children[0].get_entity().entity_id;
    assert!(
        baby_updates(&java.packets(), child_runtime, true) > 0,
        "egg baby metadata before extra ticks"
    );
    cancel.store(true, Relaxed);
    interact(scenario, java, parent.get_entity().entity_id);
    assert_eq!(
        java.player.inventory.get_stack(0).item_count,
        1,
        "cancelled egg conserved"
    );
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|e| e.cast_any().is::<VillagerEntity>())
            .count(),
        2,
        "cancelled egg admitted no child"
    );
    cancel.store(false, Relaxed);
    child_id
}

pub(super) fn summon_cure_pair(scenario: &Scenario, world: &World) -> NbtCompound {
    let cure_fixture = villager_fixture(Uuid::new_v4(), false);
    for (index, baby) in [false, true].into_iter().enumerate() {
        let mut record = cure_fixture.clone();
        record.put_uuid("UUID", Uuid::new_v4());
        record.put_bool("IsBaby", baby);
        record.put_int("ConversionTime", 1);
        record.child_tags.remove("Brain");
        scenario.command(&format!(
            "summon minecraft:zombie_villager {} 64 5 {record}",
            5 + index
        ));
    }
    for entity in world
        .entities
        .load()
        .iter()
        .filter(|e| e.get_entity().entity_type == &EntityType::ZOMBIE_VILLAGER)
    {
        entity.get_entity().age.store(8123, Relaxed);
    }
    cure_fixture
}

pub(super) fn cured_ages(
    scenario: &Scenario,
    world: &World,
    id: Uuid,
    child_id: Uuid,
) -> Vec<(Uuid, i32)> {
    let cured: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter(|e| {
            e.cast_any().is::<VillagerEntity>()
                && ![id, child_id].contains(&e.get_entity().entity_uuid)
        })
        .cloned()
        .collect();
    assert_eq!(cured.len(), 2, "real cure completion");
    let mut cured_ids = Vec::new();
    for entity in cured {
        let uuid = entity.get_entity().entity_uuid;
        let initial_age = age(scenario, uuid);
        assert!(
            [0, -24000].contains(&initial_age),
            "cure copied elapsed age"
        );
        cured_ids.push((uuid, initial_age));
    }
    assert!(
        cured_ids.iter().any(|(_, a)| *a == -24000),
        "baby cure became adult"
    );
    assert!(
        cured_ids.iter().any(|(_, a)| *a == 0),
        "adult cure became baby"
    );
    cured_ids
}

pub(super) async fn restart_age_twice(
    mut scenario: Scenario,
    ids: &[Uuid; 8],
    records: &[NbtCompound],
) -> Scenario {
    let mut previous = None;
    let mut cooldown = 199;
    for _ in 0..2 {
        scenario = scenario.restart().await;
        let mut image = restart_age_image(&scenario, ids, records, cooldown).await;
        // Only this cooldown changes on the explicit live tick below; all marker and
        // preservation fields must remain idempotent across both migrations.
        image[3][0] = None;
        if let Some(previous) = previous {
            assert_eq!(image, previous, "second migration is idempotent");
        }
        previous = Some(image);
        let (_, initial) = activate(&scenario, "AgeRestart").await;
        assert_eq!(
            initial.get(&ids[3]).unwrap().age,
            cooldown,
            "cooldown has no activation/offline step"
        );
        scenario.ticks(1).await;
        cooldown -= 1;
        assert_eq!(
            age(&scenario, ids[3]),
            cooldown,
            "restarted locked cooldown has one live step"
        );
        for id in [ids[6], ids[7]] {
            with_villager(&scenario, id, |v| assert!(v.biological_age().quarantined));
        }
    }
    scenario
}
