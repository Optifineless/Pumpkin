//! Age assertions run through world ticks, watcher activation, commands and entity packets.
mod callbacks;
mod fixtures;
mod navigation;
use super::{villager_persistence::*, *};
use crate::{
    entity::passive::villager::VillagerEntity,
    net::{
        bedrock::combat_test_support::TestBedrockPlayer, java::sound_test_support::decode_sounds,
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::entity::entity_spawn::EntitySpawnEvent,
    },
};
use fixtures::{
    activate, age_records, baby_updates, bedrock, bedrock_ages, cured_ages, real_egg,
    restart_age_twice, summon_cure_pair,
};
use pumpkin_data::{damage::DamageType, entity::EntityPose, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    codec::var_int::VarInt, java::server::play::SInteract, ser::NetworkReadExt,
};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

const SCHEMA: &str = "murgicraft:VillagerAgeSchema";

fn fixture(id: Uuid, age: i32, locked: bool) -> NbtCompound {
    let mut nbt = villager_fixture(id, false);
    nbt.put_int("Age", age);
    nbt.put_int("ForcedAge", 17);
    nbt.put_bool("AgeLocked", locked);
    nbt
}

fn age(scenario: &Scenario, id: Uuid) -> i32 {
    with_villager(scenario, id, |v| v.biological_age().age)
}

async fn bounded(body: impl Future<Output = ()>) {
    tokio::time::timeout(Duration::from_secs(90), run(body))
        .await
        .unwrap();
}

#[tokio::test]
async fn villager_s6_world_tick_uses_one_biological_step() {
    bounded(async {
        let scenario = Scenario::new().await;
        let ids: [_; 6] = std::array::from_fn(|_| Uuid::new_v4());
        let mut records = vec![
            fixture(ids[0], -2, false),
            fixture(ids[1], 2, false),
            fixture(ids[2], 0, false),
            fixture(ids[3], -2, true),
            fixture(ids[4], 2, true),
            fixture(ids[5], -2, false),
        ];
        records[5].put_float("Health", 0.0);
        seed(&scenario, records).await;
        let (_, initial) = activate(&scenario, "AgeSteps").await;
        scenario.ticks(1).await;
        for (id, expected) in ids.into_iter().zip([-2, 2, 0, -2, 2, -2]) {
            assert_eq!(
                initial.get(&id).unwrap().age,
                expected,
                "literal activation age"
            );
        }
        for (id, expected) in ids.into_iter().zip([-1, 1, 0, -2, 1, -2]) {
            assert_eq!(
                age(&scenario, id),
                expected,
                "first literal biological step"
            );
            with_villager(&scenario, id, |v| {
                assert_eq!(v.get_entity().age.load(Relaxed), 201);
            });
        }
        scenario.ticks(1).await;
        for (id, expected) in ids.into_iter().zip([0, 0, 0, -2, 0, -2]) {
            assert_eq!(
                age(&scenario, id),
                expected,
                "second literal biological step"
            );
            with_villager(&scenario, id, |v| {
                assert_eq!(v.get_entity().age.load(Relaxed), 202);
            });
        }
        let world = scenario.world(&Dimension::OVERWORLD);
        let retired = world.get_entity_by_uuid(ids[3]).unwrap();
        retired.get_entity().remove();
        scenario.ticks(1).await;
        let retired = retired.cast_any().downcast_ref::<VillagerEntity>().unwrap();
        assert_eq!(retired.biological_age().age, -2);
        assert!(
            !retired.set_biological_age(10),
            "retired owner accepted a write"
        );
        assert!(age(&scenario, ids[2]) == 0, "zero/cadence control");
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_s6_growth_publishes_metadata_geometry_and_activity() {
    bounded(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let record = fixture(id, -1, false);
        seed(&scenario, vec![record.clone()]).await;
        let mut bedrock = bedrock(&scenario).await;
        let (java, initial) = activate(&scenario, "AgeGrowth").await;
        let dimensions = with_villager(&scenario, id, navigation::watch);
        scenario.ticks(1).await;
        assert_eq!(
            dimensions.load(),
            <(f32, f32)>::from(EntityType::VILLAGER.dimension),
            "growth navigation dimensions"
        );
        let first = *initial.get(&id).unwrap();
        assert_eq!(
            (first.age, first.width, first.height),
            (-1, 0.49, 0.98),
            "initial baby body"
        );
        assert_eq!(first.eye, f64::from(0.63f32), "initial baby eye");
        let world = java.player.world();
        let villager = world.get_entity_by_uuid(id).unwrap();
        let base = villager.get_entity();
        let runtime = base.entity_id;
        let packets = java.packets();
        assert!(
            baby_updates(&packets, runtime, true) >= 1,
            "initial Java baby metadata"
        );
        assert_eq!(
            baby_updates(&packets, runtime, false),
            1,
            "one false Java growth update"
        );
        assert_eq!(age(&scenario, id), 0);
        assert_eq!(base.width(), EntityType::VILLAGER.dimension[0]);
        assert_eq!(base.height(), EntityType::VILLAGER.dimension[1]);
        assert_eq!(
            base.get_eye_height(),
            f64::from(EntityType::VILLAGER.eye_height)
        );
        assert_eq!(
            base.bounding_box.load().max.y - base.bounding_box.load().min.y,
            f64::from(EntityType::VILLAGER.dimension[1])
        );
        let bedrock_packets = bedrock.take_packets();
        assert!(
            bedrock_ages(&bedrock_packets, runtime, true).contains(&(true, 0.49, 0.98)),
            "initial Bedrock baby metadata/geometry"
        );
        assert_eq!(
            bedrock_ages(&bedrock_packets, runtime, false)
                .iter()
                .filter(|value| **value
                    == (
                        false,
                        EntityType::VILLAGER.dimension[0],
                        EntityType::VILLAGER.dimension[1]
                    ))
                .count(),
            1,
            "one false Bedrock growth update"
        );
        let mut preserved = NbtCompound::new();
        villager.write_nbt(&mut preserved);
        for key in ["Offers", "HomeX", "HomeY", "HomeZ", "Brain"] {
            assert_eq!(preserved.get(key), record.get(key), "growth retained {key}");
        }
        verify_trade_sound_and_wake(&scenario, &java, id, &villager, &mut bedrock, &dimensions);
        verify_job_and_social_eligibility(&scenario, &java, id).await;
        let cow = Scenario::mob(&world, &EntityType::COW, SPAWN);
        assert_eq!(cow.get_entity().width(), EntityType::COW.dimension[0]);
        cow.get_entity().set_pose(EntityPose::Sleeping);
        cow.get_entity().set_pose(EntityPose::Standing);
        assert_eq!(
            (cow.get_entity().width(), cow.get_entity().height()),
            (0.6, 1.8),
            "nonvillager pose fallback"
        );
        bedrock.close().await;
        scenario.stop().await;
    })
    .await;
}

fn verify_trade_sound_and_wake(
    scenario: &Scenario,
    java: &Session,
    id: Uuid,
    villager: &Arc<dyn EntityBase>,
    bedrock: &mut TestBedrockPlayer,
    dimensions: &Arc<crossbeam::atomic::AtomicCell<(f32, f32)>>,
) {
    let base = villager.get_entity();
    let runtime = base.entity_id;
    java.player.get_entity().set_pos(base.pos.load());
    villager.damage(villager.as_ref(), 1.0, DamageType::GENERIC);
    assert!(
        decode_sounds(&java.packets())
            .iter()
            .any(
                |sound| sound.sound_id == pumpkin_data::sound::Sound::EntityVillagerHurt as u16
                    && (0.8..=1.2).contains(&sound.pitch)
            ),
        "adult sound query"
    );
    interact(scenario, java, runtime);
    assert!(
        java.packets().iter().any(|p| {
            let mut b = p.as_ref();
            b.get_var_int().unwrap().0 == pumpkin_data::packet::clientbound::play::MERCHANT_OFFERS.0
        }),
        "adult trade eligibility"
    );
    java.player.close_handled_screen();
    scenario.command(&format!("data merge entity {id} {{Age:-20,AgeLocked:1b}}"));
    assert_eq!(age(scenario, id), -20, "reverse command age route");
    assert_eq!(
        (base.width(), base.height(), base.get_eye_height()),
        (0.49, 0.98, f64::from(0.63f32)),
        "reverse baby geometry/eye"
    );
    base.send_dirty_entity_data();
    assert!(
        baby_updates(&java.packets(), runtime, true) > 0,
        "reverse Java baby update"
    );
    assert!(
        bedrock_ages(&bedrock.take_packets(), runtime, false).contains(&(true, 0.49, 0.98)),
        "reverse Bedrock baby update"
    );
    interact(scenario, java, runtime);
    assert!(
        !java.packets().iter().any(|packet| {
            let mut b = packet.as_ref();
            b.get_var_int().unwrap().0 == pumpkin_data::packet::clientbound::play::MERCHANT_OFFERS.0
        }),
        "baby trade eligibility used elapsed age"
    );
    villager
        .get_living_entity()
        .unwrap()
        .hurt_cooldown
        .store(0, Relaxed);
    villager.damage(villager.as_ref(), 2.0, DamageType::GENERIC);
    let sounds = decode_sounds(&java.packets());
    assert!(
        sounds.iter().any(
            |s| s.sound_id == pumpkin_data::sound::Sound::EntityVillagerHurt as u16
                && (1.3..=1.7).contains(&s.pitch)
        ),
        "baby sound query"
    );
    verify_sleep_wake_preservation(villager, dimensions);
}

fn verify_sleep_wake_preservation(
    villager: &Arc<dyn EntityBase>,
    dimensions: &Arc<crossbeam::atomic::AtomicCell<(f32, f32)>>,
) {
    let base = villager.get_entity();
    let mut before = NbtCompound::new();
    villager.write_nbt(&mut before);
    base.set_pose(EntityPose::Sleeping);
    assert_eq!(
        (base.width(), base.height(), base.get_eye_height()),
        (0.2, 0.2, f64::from(0.2f32))
    );
    base.set_pose(EntityPose::Standing);
    assert_eq!(
        (base.width(), base.height(), base.get_eye_height()),
        (0.49, 0.98, f64::from(0.63f32)),
        "wake retained biological size"
    );
    assert_eq!(
        dimensions.load(),
        (0.49, 0.98),
        "wake navigation dimensions"
    );
    let mut after = NbtCompound::new();
    villager.write_nbt(&mut after);
    for key in ["Brain", "Offers", "HomeX", "HomeY", "HomeZ"] {
        assert_eq!(after.get(key), before.get(key), "sleep/wake retained {key}");
    }
}

async fn verify_job_and_social_eligibility(scenario: &Scenario, java: &Session, id: Uuid) {
    use crate::entity::ai::brain::memory::types;
    let world = java.player.world();
    let entity = world.get_entity_by_uuid(id).unwrap();
    let position = BlockPos::new(12, 64, 10);
    world
        .level
        .set_block_state(&position, Block::LECTERN.default_state.id);
    world
        .villager_poi
        .lock()
        .unwrap()
        .update_block(position, &Block::LECTERN);
    scenario.command(&format!(
        "data merge entity {id} {{Age:0,JobSiteX:12,JobSiteY:64,JobSiteZ:10}}"
    ));
    assert_eq!(
        entity.get_job_site_pos(),
        Some(position),
        "adult job eligibility"
    );
    scenario.command(&format!("data merge entity {id} {{Age:-20,AgeLocked:1b}}"));
    assert_eq!(
        entity.get_job_site_pos(),
        None,
        "baby job eligibility used elapsed age"
    );
    scenario.command("summon minecraft:villager 10 64 11 {NoAI:1b}");
    scenario.command(&format!("data merge entity {id} {{NoAI:0b}}"));
    let set_time = |age| {
        let mut time = world.level_time.lock().unwrap();
        time.world_age = age;
        time.time_of_day = 1000;
    };
    set_time(1199);
    scenario.ticks(1).await;
    let villager = entity.cast_any().downcast_ref::<VillagerEntity>().unwrap();
    assert!(
        !villager
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .has_memory_value(types::INTERACTION_TARGET.id()),
        "baby social eligibility used elapsed age"
    );
    scenario.command(&format!("data merge entity {id} {{Age:0}}"));
    set_time(1219);
    scenario.ticks(1).await;
    assert!(
        villager
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .has_memory_value(types::INTERACTION_TARGET.id()),
        "adult social eligibility"
    );
}

fn interact(scenario: &Scenario, player: &Session, id: i32) {
    player.send(
        &scenario.server,
        &SInteract {
            entity_id: VarInt(id),
            r#type: VarInt(2),
            target_position: Some(Vector3::new(0.0, 0.0, 0.0)),
            hand: Some(VarInt(0)),
            sneaking: false,
        },
    );
}

#[tokio::test]
async fn villager_s6_age_survives_activation_retirement_and_two_restarts() {
    bounded(async {
        let mut scenario = Scenario::new().await;
        let ids: [_; 8] = std::array::from_fn(|_| Uuid::new_v4());
        let records = age_records(&ids);
        seed(&scenario, records.clone()).await;
        let (java, initial) = activate(&scenario, "AgeRestart").await;
        for (index, id) in ids.into_iter().enumerate() {
            with_villager(&scenario, id, |villager| {
                assert_eq!(
                    villager.biological_age().had_schema,
                    [5, 7].contains(&index),
                    "missing and new age markers remain distinguishable"
                );
            });
        }
        // Public full-NBT loading on an existing legacy owner must ignore its elapsed 200 ticks.
        scenario
            .world(&Dimension::OVERWORLD)
            .get_entity_by_uuid(ids[4])
            .unwrap()
            .read_nbt_non_mut(&records[4]);
        scenario.ticks(1).await;
        assert_eq!(
            initial.get(&ids[0]).unwrap().age,
            -1,
            "initial image precedes chosen live tick"
        );
        assert_eq!(age(&scenario, ids[0]), 0, "first live tick baby boundary");
        assert_eq!(
            age(&scenario, ids[1]),
            0,
            "first live tick cooldown boundary"
        );
        assert_eq!(
            age(&scenario, ids[4]),
            0,
            "missing Age is adult, not elapsed cooldown"
        );
        for id in [ids[6], ids[7]] {
            with_villager(&scenario, id, |v| {
                assert!(v.biological_age().quarantined);
                assert!(!v.set_biological_age(-1));
                assert!(!v.is_age_ready());
            });
        }
        assert_eq!(
            age(&scenario, ids[3]),
            199,
            "initial locked cooldown live step"
        );
        // Normal watcher departure drives retirement; retained old Arcs must stay disabled.
        let old = scenario
            .world(&Dimension::OVERWORLD)
            .get_entity_by_uuid(ids[2])
            .unwrap();
        let cooldown_owner = scenario
            .world(&Dimension::OVERWORLD)
            .get_entity_by_uuid(ids[3])
            .unwrap();
        scenario.command("tp AgeRestart 512 64 0");
        java.loaded(&scenario.server);
        scenario
            .until("age departure watcher completion", || {
                scenario.server.tasks.is_empty()
            })
            .await;
        scenario.ticks(1).await;
        assert!(old.get_entity().is_removed(), "normal chunk retirement");
        assert_eq!(
            cooldown_owner
                .cast_any()
                .downcast_ref::<VillagerEntity>()
                .unwrap()
                .biological_age()
                .age,
            199,
            "retirement occurs before departure simulation"
        );
        assert!(
            !old.cast_any()
                .downcast_ref::<VillagerEntity>()
                .unwrap()
                .set_baby(false)
        );
        scenario = restart_age_twice(scenario, &ids, &records).await;
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn villager_s6_real_summon_egg_and_cure_use_biological_age() {
    bounded(async {
        let mut scenario = Scenario::new().await;
        let java = scenario.join(Uuid::new_v4(), "AgeCreators").await;
        let world = java.player.world();
        let id = Uuid::new_v4();
        let mut uuid_tag = Vec::new();
        for part in id.as_u128().to_be_bytes().as_chunks::<4>().0 {
            uuid_tag.push(i32::from_be_bytes(*part).to_string());
        }
        scenario.command(&format!(
            "summon minecraft:villager 4 64 4 {{UUID:[I;{}],Age:-2,ForcedAge:7,NoAI:1b}}",
            uuid_tag.join(",")
        ));
        assert_eq!(
            age(&scenario, id),
            -2,
            "summon NBT biological age before a tick"
        );
        let parent = world.get_entity_by_uuid(id).unwrap();
        assert!(
            baby_updates(&java.packets(), parent.get_entity().entity_id, true) > 0,
            "summon baby metadata before extra ticks"
        );
        let child_id = real_egg(&scenario, &java, &parent);
        let cure_fixture = summon_cure_pair(&scenario, &world);
        scenario.ticks(1).await;
        let mut cured_ids = cured_ages(&scenario, &world, id, child_id);
        for _ in 0..2 {
            scenario = scenario.restart().await;
            for (uuid, expected) in &cured_ids {
                let saved = saved_record(&scenario, *uuid).await;
                assert_eq!(saved.get_int("Age"), Some(*expected));
                assert_eq!(
                    saved.get("Offers"),
                    cure_fixture.get("Offers"),
                    "cure retained remembered offers"
                );
                assert_eq!(saved.get("Gossips"), cure_fixture.get("Gossips"));
                assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
                assert_eq!(
                    saved
                        .get_compound("VillagerData")
                        .unwrap()
                        .get_string("type"),
                    Some("minecraft:taiga")
                );
                assert_eq!(
                    saved
                        .get_compound("VillagerData")
                        .unwrap()
                        .get_string("profession"),
                    Some("minecraft:librarian")
                );
            }
            scenario.join(Uuid::new_v4(), "AgeCreators").await;
            for (uuid, expected) in &mut cured_ids {
                *expected = age(&scenario, *uuid);
            }
        }
        scenario.stop().await;
    })
    .await;
}
