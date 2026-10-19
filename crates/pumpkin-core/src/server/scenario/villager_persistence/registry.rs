use super::*;

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
