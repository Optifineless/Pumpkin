use super::*;
use crate::entity::mob::zombie::zombie_villager::ZombieVillagerEntity;
use pumpkin_protocol::codec::var_int::VarInt;

const CONVERSION_TICKS: i32 = 600_000;

fn zombie_fixture(id: Uuid, legacy: bool, converting: bool) -> NbtCompound {
    let mut fixture = villager_fixture(id, legacy);
    fixture.put_string("id", "minecraft:zombie_villager".into());
    fixture.put_int(
        "ConversionTime",
        if converting { CONVERSION_TICKS } else { -1 },
    );
    if converting {
        fixture.put_uuid("ConversionPlayer", TARGET);
    }
    fixture
}

fn with_zombie<T>(
    scenario: &Scenario,
    id: Uuid,
    read: impl FnOnce(&ZombieVillagerEntity) -> T,
) -> T {
    let world = scenario.world(&Dimension::OVERWORLD);
    let entity = world.get_entity_by_uuid(id).unwrap();
    read(
        entity
            .cast_any()
            .downcast_ref::<ZombieVillagerEntity>()
            .unwrap(),
    )
}

fn assert_zombie_fields(saved: &NbtCompound, fixture: &NbtCompound, converting: bool) {
    for field in ["Offers", "Gossips", "Xp", "ConversionPlayer"] {
        assert_eq!(
            saved.get(field),
            fixture.get(field),
            "unrelated zombie villager field {field} changed"
        );
    }
    let time = saved.get_int("ConversionTime").unwrap();
    if converting {
        // Joining advances real ticks; nearby beds may also speed conversion up.
        assert!(time > 0 && time <= CONVERSION_TICKS);
    } else {
        assert_eq!(time, -1);
    }
}

#[tokio::test]
async fn zombie_villager_vanilla_legacy_unknown_and_carried_fields_survive_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut unknown = zombie_fixture(ids[2], false, true);
        let mut data = NbtCompound::new();
        data.put_string("type", "future:alpine".into());
        data.put_string("profession", "future:beekeeper".into());
        data.put_int("level", 4);
        unknown.put_compound("VillagerData", data.clone());
        let fixtures = [
            zombie_fixture(ids[0], false, true),
            zombie_fixture(ids[1], true, false),
            unknown,
        ];
        seed(&scenario, fixtures.to_vec()).await;
        scenario.join(Uuid::new_v4(), "SavedZombieVillagers").await;
        for cycle in 0..=2 {
            for (index, id) in ids.into_iter().enumerate() {
                with_zombie(&scenario, id, |zombie| {
                    let data = zombie.get_villager_data();
                    assert_eq!(
                        data.r#type.0,
                        if index == 2 {
                            VillagerType::Plains
                        } else {
                            VillagerType::Taiga
                        } as i32
                    );
                    assert_eq!(
                        data.profession.0,
                        if index == 2 {
                            VillagerProfession::None
                        } else {
                            VillagerProfession::Librarian
                        } as i32
                    );
                    assert_eq!(data.level.0, if index == 2 { 4 } else { 3 });
                    assert_eq!(zombie.is_converting(), index != 1);
                });
            }
            if cycle < 2 {
                scenario = scenario.restart().await;
                for (index, id) in ids.into_iter().enumerate() {
                    let saved = saved_record(&scenario, id).await;
                    if index == 2 {
                        assert_eq!(saved.get_compound("VillagerData"), Some(&data));
                    } else {
                        let encoded = saved.get_compound("VillagerData").unwrap();
                        assert_eq!(encoded.get_string("type"), Some("minecraft:taiga"));
                        assert_eq!(
                            encoded.get_string("profession"),
                            Some("minecraft:librarian")
                        );
                        assert_eq!(encoded.get_int("level"), Some(3));
                        for key in ["Type", "Profession", "Level"] {
                            assert!(encoded.get(key).is_none(), "legacy key {key} was written");
                        }
                    }
                    assert_zombie_fields(&saved, &fixtures[index], index != 1);
                }
                scenario.join(Uuid::new_v4(), "SavedZombieVillagers").await;
            }
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn zombie_villager_real_profession_replaces_unknown_after_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = zombie_fixture(id, false, false);
        let mut data = NbtCompound::new();
        data.put_string("type", "future:alpine".into());
        data.put_string("profession", "future:beekeeper".into());
        data.put_int("level", 4);
        fixture.put_compound("VillagerData", data);
        seed(&scenario, vec![fixture.clone()]).await;
        scenario.join(Uuid::new_v4(), "ChangedZombieVillager").await;
        with_zombie(&scenario, id, |zombie| {
            let mut data = zombie.get_villager_data();
            assert_eq!(data.profession.0, VillagerProfession::None as i32);
            data.profession = VarInt(VillagerProfession::Librarian as i32);
            data.level = VarInt(5);
            zombie.set_villager_data(data);
        });
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            let data = saved.get_compound("VillagerData").unwrap();
            assert_eq!(data.get_string("type"), Some("future:alpine"));
            assert_eq!(data.get_string("profession"), Some("minecraft:librarian"));
            assert_eq!(data.get_int("level"), Some(5));
            assert_zombie_fields(&saved, &fixture, false);
            scenario.join(Uuid::new_v4(), "ChangedZombieVillager").await;
            with_zombie(&scenario, id, |zombie| {
                let data = zombie.get_villager_data();
                assert_eq!(data.profession.0, VillagerProfession::Librarian as i32);
                assert_eq!(data.r#type.0, VillagerType::Plains as i32);
                assert_eq!(data.level.0, 5);
            });
        }
        scenario.stop().await;
    })
    .await;
}

fn assert_cured_librarian(scenario: &Scenario, id: Uuid) {
    with_villager(scenario, id, |villager| {
        let data = *villager.villager_data.lock().unwrap();
        assert_eq!(data.profession.0, VillagerProfession::Librarian as i32);
        assert_eq!(data.r#type.0, VillagerType::Taiga as i32);
        assert_eq!(data.level.0, 3);
        assert!(villager.villager_data_finalized.load(Ordering::Relaxed));
        assert_eq!(villager.xp.load(Ordering::Relaxed), 45);
        let offers = villager.offers.lock().unwrap();
        assert_eq!(
            offers.len(),
            1,
            "curing lost the librarian's remembered trade"
        );
        let offer = &offers[0];
        assert_eq!(offer.base_cost_a.0.item.registry_key, "emerald");
        assert_eq!(offer.base_cost_a.0.item_count, 7);
        assert_eq!(offer.output.0.item.registry_key, "bookshelf");
        assert_eq!(offer.uses, 2);
        assert_eq!(offer.special_price, -3);
        assert_eq!(offer.demand, 4);
    });
}

fn cured_villager_id(scenario: &Scenario, zombie_id: Uuid) -> Uuid {
    let world = scenario.world(&Dimension::OVERWORLD);
    assert!(world.get_entity_by_uuid(zombie_id).is_none());
    world
        .entities
        .load()
        .iter()
        .find(|entity| entity.cast_any().is::<VillagerEntity>())
        .unwrap()
        .get_entity()
        .entity_uuid
}

#[tokio::test]
async fn zombie_villager_curing_keeps_librarian_offers_after_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let zombie_id = Uuid::new_v4();
        // Legacy data isolates the cure-order regression from registry-name decoding.
        let mut fixture = zombie_fixture(zombie_id, true, false);
        fixture.put_int("ConversionTime", 1);
        seed(&scenario, vec![fixture.clone()]).await;
        scenario.join(Uuid::new_v4(), "CuredLibrarian").await;
        scenario.ticks(3).await;
        let id = cured_villager_id(&scenario, zombie_id);
        assert_cured_librarian(&scenario, id);
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            let data = saved.get_compound("VillagerData").unwrap();
            assert_eq!(data.get_string("type"), Some("minecraft:taiga"));
            assert_eq!(data.get_string("profession"), Some("minecraft:librarian"));
            assert_eq!(data.get_int("level"), Some(3));
            assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
            for key in ["Type", "Profession", "Level"] {
                assert!(data.get(key).is_none(), "legacy key {key} was written");
            }
            assert_eq!(saved.get("Offers"), fixture.get("Offers"));
            assert_eq!(saved.get("Gossips"), fixture.get("Gossips"));
            assert_eq!(saved.get("Xp"), fixture.get("Xp"));
            scenario.join(Uuid::new_v4(), "CuredLibrarian").await;
            assert_cured_librarian(&scenario, id);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn zombie_villager_curing_keeps_unknown_names_after_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let zombie_id = Uuid::new_v4();
        let mut fixture = zombie_fixture(zombie_id, false, false);
        let mut data = NbtCompound::new();
        data.put_string("type", "future:alpine".into());
        data.put_string("profession", "future:beekeeper".into());
        data.put_int("level", 4);
        fixture.put_compound("VillagerData", data.clone());
        fixture.put_int("ConversionTime", 1);
        seed(&scenario, vec![fixture.clone()]).await;
        scenario.join(Uuid::new_v4(), "CuredFutureVillager").await;
        scenario.ticks(3).await;
        let id = cured_villager_id(&scenario, zombie_id);
        for cycle in 0..=2 {
            with_villager(&scenario, id, |villager| {
                let data = *villager.villager_data.lock().unwrap();
                assert_eq!(data.profession.0, VillagerProfession::None as i32);
                assert_eq!(data.r#type.0, VillagerType::Plains as i32);
                assert_eq!(data.level.0, 4);
                assert!(villager.villager_data_finalized.load(Ordering::Relaxed));
                assert_eq!(villager.offers.lock().unwrap().len(), 1);
            });
            if cycle < 2 {
                scenario = scenario.restart().await;
                let saved = saved_record(&scenario, id).await;
                assert_eq!(saved.get_compound("VillagerData"), Some(&data));
                assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
                assert_eq!(saved.get("Offers"), fixture.get("Offers"));
                scenario.join(Uuid::new_v4(), "CuredFutureVillager").await;
            }
        }
        scenario.stop().await;
    })
    .await;
}
