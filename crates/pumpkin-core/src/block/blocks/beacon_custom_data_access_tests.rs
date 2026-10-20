use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_custom_data_snbt_string_locks_match_beacons_and_chests() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("SNBT custom data beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let expected =
            NbtTag::String("{beacon_key:1,nested:{entries:[{id:'matching entry'}]}}".into());
        let beacon = load_beacon(&fixture, custom_data_beacon(&title_tag, &expected));
        let beacon = save_reload(&fixture, beacon.as_ref());
        let mut saved = NbtCompound::new();
        beacon.write_internal(&mut saved);
        assert_eq!(
            saved
                .get_compound("lock")
                .unwrap()
                .get_compound("predicates")
                .unwrap()
                .get("minecraft:custom_data"),
            Some(&expected),
            "the vanilla SNBT string form must survive beacon save and reload",
        );

        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            custom_data_key(NbtTag::Int(2)),
            ItemStack::EMPTY.clone(),
            "wrong key for SNBT-string custom_data",
        );
        fixture
            .player
            .player
            .inventory
            .set_stack(0, custom_data_key(NbtTag::Int(1)));
        fixture.player.player.inventory.set_stack(
            pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT,
            ItemStack::EMPTY.clone(),
        );
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);

        open_custom_data_chest(&mut fixture, &expected);
        fixture.finish().await;
    })
    .await
    .expect("bounded SNBT-string custom_data lock regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_custom_data_malformed_snbt_denies_without_panicking() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Malformed SNBT beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let beacon = load_beacon(
            &fixture,
            custom_data_beacon(&title_tag, &NbtTag::String("{beacon_key:1".into())),
        );
        let _beacon = save_reload(&fixture, beacon.as_ref());
        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            custom_data_key(NbtTag::Int(1)),
            ItemStack::EMPTY.clone(),
            "malformed SNBT-string custom_data",
        );
        fixture.finish().await;
    })
    .await
    .expect("bounded malformed custom_data SNBT regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_custom_data_deep_malformed_snbt_denies_and_round_trips() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Deep malformed SNBT beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let raw_snbt = format!("{{x:{}", "[".repeat(60_000));
        assert_eq!(raw_snbt.len(), 60_003);
        let expected = NbtTag::String(raw_snbt.clone().into());
        let beacon = load_beacon(&fixture, custom_data_beacon(&title_tag, &expected));

        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            custom_data_key(NbtTag::Int(1)),
            ItemStack::EMPTY.clone(),
            "deep malformed SNBT-string custom_data",
        );

        let mut saved = NbtCompound::new();
        beacon.write_internal(&mut saved);
        assert_eq!(
            saved
                .get_compound("lock")
                .unwrap()
                .get_compound("predicates")
                .unwrap()
                .get("minecraft:custom_data"),
            Some(&expected),
            "use must not rewrite the malformed saved SNBT string",
        );

        let reloaded = load_beacon(&fixture, saved);
        let mut saved_again = NbtCompound::new();
        reloaded.write_internal(&mut saved_again);
        let persisted = saved_again
            .get_compound("lock")
            .unwrap()
            .get_compound("predicates")
            .unwrap()
            .get("minecraft:custom_data")
            .unwrap();
        let NbtTag::String(persisted) = persisted else {
            panic!("saved custom_data must remain an SNBT string");
        };
        assert_eq!(
            persisted.as_bytes(),
            raw_snbt.as_bytes(),
            "reload must preserve the raw malformed SNBT bytes exactly",
        );
        fixture.finish().await;
    })
    .await
    .expect("bounded deep malformed custom_data SNBT regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_empty_custom_data_predicate_matches_absent_components() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Empty custom data beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let beacon = load_beacon(
            &fixture,
            custom_data_beacon(&title_tag, &NbtTag::Compound(NbtCompound::new())),
        );
        let _beacon = save_reload(&fixture, beacon.as_ref());

        fixture
            .player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::PAPER));
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);

        fixture
            .player
            .player
            .inventory
            .set_stack(0, ItemStack::EMPTY.clone());
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);
        fixture.finish().await;

        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Nonempty custom data beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let beacon = load_beacon(
            &fixture,
            custom_data_beacon(&title_tag, &NbtTag::Compound(expected_custom_data())),
        );
        let _beacon = save_reload(&fixture, beacon.as_ref());
        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            ItemStack::new(1, &Item::PAPER),
            ItemStack::EMPTY.clone(),
            "nonempty expected data with absent component",
        );
        fixture.finish().await;
    })
    .await
    .expect("bounded absent custom_data component regression timed out");
}
