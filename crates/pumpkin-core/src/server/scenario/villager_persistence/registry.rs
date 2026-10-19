use super::*;
use pumpkin_protocol::codec::var_int::VarInt;

fn unknown_fixture(id: Uuid) -> NbtCompound {
    let mut fixture = villager_fixture(id, false);
    let mut data = NbtCompound::new();
    data.put_string("type", "future:alpine".into());
    data.put_string("profession", "future:beekeeper".into());
    data.put_int("level", 4);
    fixture.put_compound("VillagerData", data);
    fixture
}

async fn changed_unknown_data_survives_two_restarts(direct: bool) {
    let mut scenario = Scenario::new().await;
    let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    seed(&scenario, ids.into_iter().map(unknown_fixture).collect()).await;
    scenario
        .join(Uuid::new_v4(), "ChangedFutureVillagers")
        .await;
    for (index, id) in ids.into_iter().enumerate() {
        with_villager(&scenario, id, |villager| {
            let mut data = *villager.villager_data.lock().unwrap();
            assert_eq!(data.profession.0, VillagerProfession::None as i32);
            assert_eq!(data.r#type.0, VillagerType::Plains as i32);
            data.level = VarInt(5);
            match index {
                0 => data.profession = VarInt(VillagerProfession::Librarian as i32),
                2 => data.r#type = VarInt(VillagerType::Desert as i32),
                _ => {}
            }
            if direct {
                // Both Wasm hosts change this public mutex without calling the setter.
                *villager.villager_data.lock().unwrap() = data;
            } else {
                villager.set_villager_data(data);
            }
        });
    }
    for _ in 0..2 {
        scenario = scenario.restart().await;
        for (index, id) in ids.into_iter().enumerate() {
            let saved = saved_record(&scenario, id).await;
            let data = saved.get_compound("VillagerData").unwrap();
            assert_eq!(
                data.get_string("type"),
                Some(if index == 2 {
                    "minecraft:desert"
                } else {
                    "future:alpine"
                })
            );
            assert_eq!(
                data.get_string("profession"),
                Some(if index == 0 {
                    "minecraft:librarian"
                } else {
                    "future:beekeeper"
                })
            );
            assert_eq!(data.get_int("level"), Some(5));
        }
        scenario
            .join(Uuid::new_v4(), "ChangedFutureVillagers")
            .await;
        for (index, id) in ids.into_iter().enumerate() {
            with_villager(&scenario, id, |villager| {
                let data = *villager.villager_data.lock().unwrap();
                assert_eq!(
                    data.profession.0,
                    if index == 0 {
                        VillagerProfession::Librarian
                    } else {
                        VillagerProfession::None
                    } as i32
                );
                assert_eq!(
                    data.r#type.0,
                    if index == 2 {
                        VillagerType::Desert
                    } else {
                        VillagerType::Plains
                    } as i32
                );
                assert_eq!(data.level.0, 5);
            });
        }
    }
    scenario.stop().await;
}

#[tokio::test]
async fn villager_setter_replaces_changed_unknown_names_after_two_restarts() {
    run(changed_unknown_data_survives_two_restarts(false)).await;
}

#[tokio::test]
async fn villager_direct_write_replaces_changed_unknown_names_after_two_restarts() {
    run(changed_unknown_data_survives_two_restarts(true)).await;
}

#[tokio::test]
async fn villager_unknown_registry_names_survive_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = villager_fixture(id, false);
        let mut data = NbtCompound::new();
        data.put_string("type", "future:alpine".into());
        data.put_string("profession", "future:beekeeper".into());
        data.put_int("level", 4);
        fixture.put_compound("VillagerData", data.clone());
        seed(&scenario, vec![fixture]).await;
        scenario.join(Uuid::new_v4(), "FutureVillager").await;
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            assert_eq!(saved.get_compound("VillagerData"), Some(&data));
            assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
            scenario.join(Uuid::new_v4(), "FutureVillager").await;
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_finalized_flag_or_decoded_data_finalizes_after_chunk_activation() {
    run(async {
        let mut scenario = Scenario::new().await;
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        let mut explicit = villager_fixture(ids[0], false);
        explicit.child_tags.remove("VillagerData");
        let mut decoded = villager_fixture(ids[1], false);
        decoded.put_bool("VillagerDataFinalized", false);
        seed(&scenario, vec![explicit, decoded]).await;
        scenario.join(Uuid::new_v4(), "FinalizedVillagers").await;
        for _ in 0..2 {
            scenario = scenario.restart().await;
            for (index, id) in ids.iter().enumerate() {
                let saved = saved_record(&scenario, *id).await;
                assert_eq!(saved.get_bool("VillagerDataFinalized"), Some(true));
                let data = saved.get_compound("VillagerData").unwrap();
                if index == 0 {
                    assert_eq!(data.get_string("type"), Some("minecraft:plains"));
                    assert_eq!(data.get_string("profession"), Some("minecraft:none"));
                    assert_eq!(data.get_int("level"), Some(1));
                } else {
                    assert_vanilla_encoding(&saved);
                }
            }
            scenario.join(Uuid::new_v4(), "FinalizedVillagers").await;
        }
        scenario.stop().await;
    })
    .await;
}
