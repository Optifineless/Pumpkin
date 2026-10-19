#![expect(
    clippy::unwrap_used,
    reason = "Production bossbar regression assertions"
)]

use super::*;
use crate::{
    server::scenario::{Scenario, Session, run},
    world::bossbar::recipient_test_support::{Fixture, events, operations, probe, transfer},
};
use pumpkin_data::dimension::Dimension;

#[tokio::test]
async fn b1020_g03_dragon_cancelled_transfer_and_custom_bar_controls() {
    run(async {
        let fixture = Fixture::new().await;
        let viewer = fixture
            .viewer(Uuid::new_v4(), "DragonCancel", &fixture.end)
            .await;
        let (_, uuid) = live_fight(&fixture);
        scan(&fixture);
        assert_eq!(operations(&events(&viewer), uuid), vec![0]);
        fixture
            .scenario
            .command("bossbar add regression:travel \"Assigned\"");
        fixture
            .scenario
            .command("bossbar set regression:travel players DragonCancel");
        let custom = events(&viewer)
            .into_iter()
            .find(|(_, op)| *op == 0)
            .unwrap()
            .0;
        let cancel =
            Arc::new(crate::world::bossbar::recipient_test_support::CancelTransfer::default());
        cancel.0.store(true, std::sync::atomic::Ordering::Relaxed);
        fixture.scenario.server.plugin_manager.register(
            cancel.clone(),
            crate::plugin::EventPriority::Normal,
            true,
        );
        let destination = fixture.scenario.world(&Dimension::OVERWORLD);
        fixture
            .scenario
            .drive(
                "cancelled world teleport",
                viewer.player.teleport_world(
                    destination.clone(),
                    crate::server::scenario::SPAWN,
                    None,
                    None,
                ),
            )
            .await;
        assert!(Arc::ptr_eq(&viewer.player.world(), &fixture.end));
        scan(&fixture);
        assert!(
            events(&viewer).is_empty(),
            "cancelled transfer changed bossbars"
        );
        cancel.0.store(false, std::sync::atomic::Ordering::Relaxed);
        transfer(
            &fixture.scenario,
            &viewer,
            &destination,
            crate::server::scenario::SPAWN,
        )
        .await;
        scan(&fixture);
        let packets = events(&viewer);
        assert!(
            operations(&packets, custom).is_empty(),
            "assigned custom bar changed on transfer"
        );
        assert!(
            fixture
                .scenario
                .server
                .bossbars
                .lock()
                .unwrap()
                .get_bossbar("regression:travel")
                .unwrap()
                .players
                .contains(&viewer.player.gameprofile.id)
        );
        fixture.scenario.stop().await;
    })
    .await;
}

fn live_fight(fixture: &Fixture) -> (Arc<dyn EntityBase>, Uuid) {
    fixture.arena();
    let dragon = Scenario::mob(
        &fixture.end,
        &EntityType::ENDER_DRAGON,
        Vector3::new(0.5, 128.0, 0.5),
    );
    let mut fight = fixture.end.dragon_fight.as_ref().unwrap().lock().unwrap();
    fight.needs_state_scanning = false;
    fight.dragon_killed = false;
    fight.previously_killed = true;
    fight.dragon_uuid = Some(dragon.get_entity().entity_uuid);
    fight.respawn_stage = None;
    fight.exit_portal_location = Some(BlockPos::new(0, 64, 0));
    fight.gateways.clear();
    (dragon, fight.bossbar_uuid)
}

fn scan(fixture: &Fixture) {
    fixture.arena();
    for _ in 0..TIME_BETWEEN_PLAYER_SCANS {
        DragonFight::tick(fixture.end.dragon_fight.as_ref().unwrap(), &fixture.end);
    }
}

fn assert_retired(fixture: &Fixture, viewer: &Session) {
    assert!(
        !fixture
            .end
            .dragon_fight
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .bossbar_players
            .iter()
            .any(|r| r.matches(&viewer.player)),
        "departed dragon recipient was not retired"
    );
}

fn update(fixture: &Fixture, dragon: &Arc<dyn EntityBase>) {
    DragonFight::with_fight(
        fixture.end.dragon_fight.as_ref().unwrap(),
        &fixture.end,
        |fight| {
            fight.update_dragon(
                &fixture.end,
                dragon.get_entity().entity_uuid,
                100.0,
                200.0,
                Some(&TextComponent::text("Changed dragon")),
            );
        },
    );
}

#[tokio::test]
async fn b1020_g03_dragon_outer_effect_batch_revalidates_delivery() {
    run(async {
        let fixture = Fixture::new().await;
        let viewer = fixture
            .viewer(Uuid::new_v4(), "BatchedDragon", &fixture.end)
            .await;
        let (_, uuid) = live_fight(&fixture);
        let (queued, receiver) = std::sync::mpsc::channel();
        let (resume, released) = std::sync::mpsc::channel();
        let world = fixture.end.clone();
        let worker = std::thread::spawn(move || {
            let batch = crate::world::guard_effects::GuardEffects::new();
            for _ in 0..TIME_BETWEEN_PLAYER_SCANS {
                DragonFight::tick(world.dragon_fight.as_ref().unwrap(), &world);
            }
            queued.send(()).unwrap();
            released
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
            drop(batch);
        });
        fixture
            .scenario
            .until("dragon batched ADD queued", || receiver.try_recv().is_ok())
            .await;
        assert!(operations(&events(&viewer), uuid).is_empty());
        transfer(
            &fixture.scenario,
            &viewer,
            &fixture.scenario.world(&Dimension::OVERWORLD),
            crate::server::scenario::SPAWN,
        )
        .await;
        scan(&fixture);
        resume.send(()).unwrap();
        fixture
            .scenario
            .drive(
                "dragon batch worker joins",
                tokio::task::spawn_blocking(move || worker.join().unwrap()),
            )
            .await
            .unwrap();
        let ops = operations(&events(&viewer), uuid);
        assert_eq!(
            ops,
            Vec::<i32>::new(),
            "batched dragon ADD became visible after source retirement: {ops:?}"
        );
        assert_retired(&fixture, &viewer);
        fixture.scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_dragon_bar_removed_after_real_cross_world_transfer() {
    run(async {
        let fixture = Fixture::new().await;
        let viewer = fixture
            .viewer(Uuid::new_v4(), "DragonTrip", &fixture.end)
            .await;
        let (dragon, uuid) = live_fight(&fixture);
        scan(&fixture);
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![0],
            "live fight never delivered ADD"
        );
        update(&fixture, &dragon);
        assert_eq!(operations(&events(&viewer), uuid), vec![2, 3]);
        transfer(
            &fixture.scenario,
            &viewer,
            &fixture.scenario.world(&Dimension::OVERWORLD),
            crate::server::scenario::SPAWN,
        )
        .await;
        scan(&fixture);
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "departed connected dragon viewer must receive exactly one REMOVE"
        );
        assert_retired(&fixture, &viewer);
        scan(&fixture);
        update(&fixture, &dragon);
        assert!(
            operations(&events(&viewer), uuid).is_empty(),
            "retired dragon recipient received stale updates"
        );
        transfer(
            &fixture.scenario,
            &viewer,
            &fixture.end,
            Vector3::new(3.5, 128.0, 3.5),
        )
        .await;
        scan(&fixture);
        assert_eq!(operations(&events(&viewer), uuid), vec![0]);
        scan(&fixture);
        assert!(operations(&events(&viewer), uuid).is_empty());
        fixture.scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_dragon_bulk_removal_after_departure() {
    run(async {
        let fixture = Fixture::new().await;
        let viewer = fixture
            .viewer(Uuid::new_v4(), "DragonBulk", &fixture.end)
            .await;
        let (dragon, uuid) = live_fight(&fixture);
        scan(&fixture);
        assert_eq!(operations(&events(&viewer), uuid), vec![0]);
        transfer(
            &fixture.scenario,
            &viewer,
            &fixture.scenario.world(&Dimension::OVERWORLD),
            crate::server::scenario::SPAWN,
        )
        .await;
        fixture.arena();
        DragonFight::with_fight(
            fixture.end.dragon_fight.as_ref().unwrap(),
            &fixture.end,
            |fight| {
                fight.set_dragon_killed(&fixture.end, dragon.get_entity().entity_uuid);
            },
        );
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "bulk cleanup lost departed dragon viewer REMOVE"
        );
        assert_retired(&fixture, &viewer);
        scan(&fixture);
        assert!(operations(&events(&viewer), uuid).is_empty());
        fixture.scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_same_uuid_rejoin_is_a_new_recipient() {
    run(async {
        let fixture = Fixture::new().await;
        let id = Uuid::new_v4();
        let a = fixture.viewer(id, "DragonRejoin", &fixture.end).await;
        let (_, uuid) = live_fight(&fixture);
        scan(&fixture);
        assert_eq!(operations(&events(&a), uuid), vec![0]);
        fixture.scenario.leave(&a).await;
        assert!(a.client().is_closed());
        let b = fixture.viewer(id, "DragonRejoin", &fixture.end).await;
        assert!(!Arc::ptr_eq(&a.player, &b.player));
        // The replacement arrives between owner scans; its UUID must not inherit A's membership.
        scan(&fixture);
        assert_eq!(
            operations(&events(&b), uuid),
            vec![0],
            "replacement session did not receive its own dragon ADD"
        );
        assert!(operations(&events(&a), uuid).is_empty());
        scan(&fixture);
        assert!(
            operations(&events(&b), uuid).is_empty(),
            "old cleanup removed the replacement bar"
        );
        fixture.scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g03_departure_during_deferred_add_cannot_publish_stale_bar() {
    run(async {
        let fixture = std::rc::Rc::new(Fixture::new().await);
        let viewer = fixture
            .viewer(Uuid::new_v4(), "DeferredDragon", &fixture.end)
            .await;
        let (dragon, uuid) = live_fight(&fixture);
        let owner = Arc::downgrade(&fixture.end);
        probe(&fixture.scenario, move |_| {
            assert!(
                owner
                    .upgrade()
                    .unwrap()
                    .dragon_fight
                    .as_ref()
                    .unwrap()
                    .try_lock()
                    .is_ok(),
                "packet callback retained fight guard"
            );
        });
        let f = fixture.clone();
        let v = viewer.clone();
        super::callback_tests::before_actions(move || {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    transfer(
                        &f.scenario,
                        &v,
                        &f.scenario.world(&Dimension::OVERWORLD),
                        crate::server::scenario::SPAWN,
                    )
                    .await;
                });
            });
        });
        scan(&fixture);
        scan(&fixture);
        update(&fixture, &dragon);
        let ops = operations(&events(&viewer), uuid);
        let mut visible = false;
        for op in &ops {
            match op {
                0 => visible = true,
                1 => visible = false,
                _ => panic!("departed recipient received stale update: {ops:?}"),
            }
        }
        assert!(!visible, "deferred source bar was never removed: {ops:?}");
        assert_retired(&fixture, &viewer);
        fixture.scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_dragon_same_world_range_exit_control() {
    run(async {
        let fixture = Fixture::new().await;
        let viewer = fixture
            .viewer(Uuid::new_v4(), "DragonRange", &fixture.end)
            .await;
        let (dragon, uuid) = live_fight(&fixture);
        scan(&fixture);
        assert_eq!(operations(&events(&viewer), uuid), vec![0]);
        viewer
            .player
            .get_entity()
            .set_pos(Vector3::new(250.0, 128.0, 0.0));
        scan(&fixture);
        assert_eq!(operations(&events(&viewer), uuid), vec![1]);
        assert_retired(&fixture, &viewer);
        viewer
            .player
            .get_entity()
            .set_pos(Vector3::new(3.5, 128.0, 3.5));
        scan(&fixture);
        assert_eq!(operations(&events(&viewer), uuid), vec![0]);
        DragonFight::with_fight(
            fixture.end.dragon_fight.as_ref().unwrap(),
            &fixture.end,
            |fight| {
                fight.set_dragon_killed(&fixture.end, dragon.get_entity().entity_uuid);
            },
        );
        assert_eq!(operations(&events(&viewer), uuid), vec![2, 1]);
        fixture.scenario.stop().await;
    })
    .await;
}
