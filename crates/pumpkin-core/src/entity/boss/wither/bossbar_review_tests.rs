use super::*;
use pumpkin_protocol::ser::NetworkReadExt;
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, AtomicUsize},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019d_wither_twice_panicking_terminal_remove_is_retried_without_mutation() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "TwiceRemove").await;
        let other = scenario.join(Uuid::new_v4(), "RemoveOther").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        assert_eq!(operations(&events(&other), uuid), vec![0, 2]);
        viewer
            .client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_4);
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let owner = Arc::downgrade(&entity);
        let id = viewer.player.gameprofile.id;
        probe(scenario, move |event| {
            let mut payload = event.payload.as_ref();
            if event.player.gameprofile.id == id
                && payload.get_uuid().unwrap() == uuid
                && payload.get_var_int().unwrap().0 == 1
            {
                assert!(!std::thread::panicking());
                assert!(
                    boss(&owner.upgrade().unwrap())
                        .bossbar_players
                        .try_lock()
                        .is_ok()
                );
                assert!(
                    observed.fetch_add(1, Ordering::Relaxed) >= 2,
                    "two-shot terminal Wither REMOVE panic"
                );
            }
        });
        entity.get_entity().remove();

        let failed = catch_unwind(AssertUnwindSafe(|| {
            let _effects = crate::world::guard_effects::GuardEffects::new();
            let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
            entity.write_nbt(&mut nbt);
            assert_eq!(
                attempts.load(Ordering::Relaxed),
                0,
                "delivery escaped the outer batch"
            );
        }));

        assert!(failed.is_err(), "terminal callbacks did not unwind");
        assert_eq!(
            operations(&events(&other), uuid),
            vec![1],
            "failed recipient stranded its successor"
        );
        let received = RefCell::new(Vec::new());
        scenario
            .until("twice-failed terminal REMOVE recovery", || {
                received
                    .borrow_mut()
                    .extend(operations(&events(&viewer), uuid));
                received.borrow().contains(&1)
            })
            .await;
        assert_eq!(*received.borrow(), vec![1]);
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        assert!(
            boss(&entity)
                .bossbar_players
                .lock()
                .unwrap()
                .recipients
                .is_empty()
        );
        tick(&entity, scenario);
        assert!(operations(&events(&viewer), uuid).is_empty());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019d_wither_persistent_remove_panic_parks_after_one_recovery() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "ParkRemove").await;
        let other = scenario.join(Uuid::new_v4(), "ParkOther").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        events(&viewer);
        events(&other);
        viewer
            .client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_4);
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let armed = Arc::new(AtomicBool::new(true));
        let failing = armed.clone();
        let id = viewer.player.gameprofile.id;
        probe(scenario, move |event| {
            let mut payload = event.payload.as_ref();
            if event.player.gameprofile.id == id
                && payload.get_uuid().unwrap() == uuid
                && payload.get_var_int().unwrap().0 == 1
            {
                assert!(!std::thread::panicking());
                observed.fetch_add(1, Ordering::Relaxed);
                assert!(
                    !failing.load(Ordering::Relaxed),
                    "persistent terminal REMOVE panic"
                );
            }
        });
        entity.get_entity().remove();
        let serialize = || entity.write_nbt(&mut pumpkin_nbt::compound::NbtCompound::new());

        assert!(catch_unwind(AssertUnwindSafe(serialize)).is_err());

        scenario
            .until("persistent REMOVE parks after bounded recovery", || {
                boss(&entity)
                    .bossbar_players
                    .try_lock()
                    .is_ok_and(|state| state.is_parked())
                    && attempts.load(Ordering::Relaxed) == 4
            })
            .await;
        assert_eq!(operations(&events(&other), uuid), vec![1]);
        assert!(operations(&events(&viewer), uuid).is_empty());
        armed.store(false, Ordering::Relaxed);
        serialize();
        assert_eq!(operations(&events(&viewer), uuid), vec![1]);
        assert_eq!(attempts.load(Ordering::Relaxed), 5);
        serialize();
        assert!(operations(&events(&viewer), uuid).is_empty());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019d_wither_callback_panic_allows_later_health_and_death_remove() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "WitherUnwind").await;
        let other = scenario.join(Uuid::new_v4(), "UnwindOther").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        assert_eq!(operations(&events(&other), uuid), vec![0, 2]);
        viewer
            .client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_4);
        let armed = Arc::new(AtomicBool::new(true));
        let once = armed.clone();
        let id = viewer.player.gameprofile.id;
        probe(scenario, move |event| {
            if event.player.gameprofile.id == id && once.swap(false, Ordering::Relaxed) {
                assert!(!std::thread::panicking());
                panic!("one-shot Wither packet delivery panic");
            }
        });

        let failed = catch_unwind(AssertUnwindSafe(|| tick(&entity, scenario)));

        assert!(
            failed.is_err(),
            "real bossbar packet callback did not unwind"
        );
        assert!(!armed.load(Ordering::Relaxed));
        events(&viewer);
        events(&other);
        tick(&entity, scenario);
        assert!(
            operations(&events(&viewer), uuid).contains(&2),
            "a panicking delivery stranded the next Wither health update"
        );
        assert!(operations(&events(&other), uuid).contains(&2));
        let terminal = Arc::new(AtomicBool::new(true));
        let once = terminal.clone();
        probe(scenario, move |event| {
            let mut payload = event.payload.as_ref();
            let sent_uuid = payload.get_uuid().unwrap();
            let operation = payload.get_var_int().unwrap().0;
            assert!(
                !(sent_uuid == uuid && operation == 1 && once.swap(false, Ordering::Relaxed)),
                "one-shot terminal Wither REMOVE panic"
            );
        });
        let failed = catch_unwind(AssertUnwindSafe(|| {
            scenario.command(&format!("kill {}", entity.get_entity().entity_uuid));
            tick(&entity, scenario);
        }));
        assert!(failed.is_err(), "terminal REMOVE callback did not unwind");
        assert!(!terminal.load(Ordering::Relaxed));
        assert_eq!(entity.get_living_entity().unwrap().health.load(), 0.0);
        assert_eq!(operations(&events(&viewer), uuid), vec![1]);
        assert_eq!(operations(&events(&other), uuid), vec![1]);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019d_wither_abandoned_removed_serialization_retries_without_owner_mutation() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "WitherAbandon").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        viewer
            .client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_4);
        probe(scenario, |_| assert!(!std::thread::panicking()));
        let owner = Arc::downgrade(&entity);
        entity.get_entity().remove();
        let failed = catch_unwind(AssertUnwindSafe(|| {
            let _effects = crate::world::guard_effects::GuardEffects::new();
            let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
            entity.write_nbt(&mut nbt);
            assert_eq!(
                nbt.get_int("Invul"),
                Some(boss(&entity).get_invulnerable_ticks())
            );
            panic!("abandon the removed owner's queued serialization cleanup");
        }));
        assert!(failed.is_err());
        assert!(
            owner.upgrade().is_some(),
            "test released the retained Wither owner"
        );
        let received = RefCell::new(Vec::new());
        scenario
            .until("abandoned Wither terminal REMOVE retry", || {
                received
                    .borrow_mut()
                    .extend(operations(&events(&viewer), uuid));
                received.borrow().contains(&1)
            })
            .await;
        assert_eq!(*received.borrow(), vec![1]);
        assert!(owner.upgrade().is_some());
        scenario.stop().await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019d_wither_twice_panicking_remove_precedes_same_session_readd() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "ReaddRemove").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        viewer
            .client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_4);
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let owner = Arc::downgrade(&entity);
        let server = Arc::downgrade(&scenario.server);
        let id = viewer.player.gameprofile.id;
        probe(scenario, move |event| {
            let mut payload = event.payload.as_ref();
            if event.player.gameprofile.id == id
                && payload.get_uuid().unwrap() == uuid
                && payload.get_var_int().unwrap().0 == 1
            {
                assert!(!std::thread::panicking());
                let owner = owner.upgrade().unwrap();
                assert!(boss(&owner).bossbar_players.try_lock().is_ok());
                let attempt = observed.fetch_add(1, Ordering::Relaxed);
                if attempt == 0 {
                    event.player.get_entity().set_pos(SPAWN);
                    owner.tick(owner.as_ref(), &server.upgrade().unwrap());
                }
                assert!(attempt >= 2, "two-shot REMOVE before re-add panic");
            }
        });
        viewer
            .player
            .get_entity()
            .set_pos(Vector3::new(80.0, 64.0, 0.0));

        assert!(catch_unwind(AssertUnwindSafe(|| tick(&entity, scenario))).is_err());

        let received = RefCell::new(Vec::new());
        scenario
            .until("failed REMOVE before returning session ADD", || {
                received
                    .borrow_mut()
                    .extend(operations(&events(&viewer), uuid));
                received.borrow().len() >= 3
            })
            .await;
        assert_eq!(
            *received.borrow(),
            vec![1, 0, 2],
            "old REMOVE hid the returning session's new bar"
        );
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![2]);
        scenario.stop().await;
    })
    .await;
}
