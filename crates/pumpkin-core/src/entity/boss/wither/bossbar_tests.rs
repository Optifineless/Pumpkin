#![expect(
    clippy::unwrap_used,
    reason = "Production lifecycle regression assertions"
)]

use super::*;
use crate::{
    server::scenario::{SPAWN, Scenario, Session, run},
    world::bossbar::recipient_test_support::{Fixture, events, operations, probe, transfer},
};
use pumpkin_data::dimension::Dimension;
use pumpkin_protocol::{VarInt, java::server::play::SClientCommand};

#[tokio::test]
async fn b1020_g03_wither_cancelled_transfer_control() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "WitherCancel").await;
        let world = viewer.player.world();
        let entity = wither(&world);
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        let cancel =
            Arc::new(crate::world::bossbar::recipient_test_support::CancelTransfer::default());
        cancel.0.store(true, Ordering::Relaxed);
        scenario
            .server
            .plugin_manager
            .register(cancel, crate::plugin::EventPriority::Normal, true);
        scenario
            .drive(
                "cancelled Wither viewer transfer",
                viewer.player.teleport_world(
                    scenario.world(&Dimension::THE_NETHER),
                    SPAWN,
                    None,
                    None,
                ),
            )
            .await;
        assert!(Arc::ptr_eq(&viewer.player.world(), &world));
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![2]);
        scenario.stop().await;
    })
    .await;
}

fn wither(world: &Arc<crate::world::World>) -> Arc<dyn EntityBase> {
    let entity = Scenario::mob(world, &EntityType::WITHER, SPAWN);
    let boss = entity.cast_any().downcast_ref::<WitherEntity>().unwrap();
    boss.set_invulnerable_ticks(100);
    entity
}

fn boss(entity: &Arc<dyn EntityBase>) -> &WitherEntity {
    entity.cast_any().downcast_ref::<WitherEntity>().unwrap()
}

fn tick(entity: &Arc<dyn EntityBase>, scenario: &Scenario) {
    entity.tick(entity.as_ref(), &scenario.server);
}

fn retired(entity: &Arc<dyn EntityBase>, viewer: &Session) {
    assert!(
        !boss(entity)
            .bossbar_players
            .lock()
            .unwrap()
            .recipients
            .iter()
            .any(|r| r.matches(&viewer.player)),
        "departed Wither recipient was not retired"
    );
}

#[tokio::test]
async fn b1020_g03_wither_discard_cleanup_on_owner_release() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "DiscardWither").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        let owner = Arc::downgrade(&entity);
        entity.get_entity().remove();
        assert!(entity.get_entity().is_removed());
        drop(entity);
        assert!(
            owner.upgrade().is_none(),
            "discarded owner remained retained"
        );
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "discarded Wither lost its viewer REMOVE"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_wither_unload_removes_departed_viewer_without_mob_tick() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "UnloadWither").await;
        let source = viewer.player.world();
        let entity = wither(&source);
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        let age = entity.get_entity().age.load(Ordering::Relaxed);
        transfer(
            scenario,
            &viewer,
            &scenario.world(&Dimension::THE_NETHER),
            SPAWN,
        )
        .await;
        scenario.ticks(1).await;
        assert!(
            entity.get_entity().is_removed(),
            "source owner was not retired by real chunk unload"
        );
        assert_eq!(
            entity.get_entity().age.load(Ordering::Relaxed),
            age,
            "owner tick hid the unload-cleanup regression"
        );
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "retired Wither serialization lost viewer REMOVE"
        );
        retired(&entity, &viewer);
        scenario.ticks(1).await;
        assert!(operations(&events(&viewer), uuid).is_empty());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_wither_cross_world_respawn_removes_departed_bar() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "WitherRespawn").await;
        let source = scenario.world(&Dimension::THE_NETHER);
        transfer(scenario, &viewer, &source, SPAWN).await;
        let entity = wither(&source);
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        scenario.command("kill WitherRespawn");
        assert_eq!(viewer.player.living_entity.health.load(), 0.0);
        viewer.send(
            &scenario.server,
            &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
        );
        scenario
            .until("Wither viewer respawn task", || {
                scenario.server.tasks.is_empty()
            })
            .await;
        assert!(Arc::ptr_eq(
            &viewer.player.world(),
            &scenario.world(&Dimension::OVERWORLD)
        ));
        assert!(viewer.player.living_entity.health.load() > 0.0);
        assert!(
            !source
                .players
                .load()
                .iter()
                .any(|p| Arc::ptr_eq(p, &viewer.player))
        );
        viewer.loaded(&scenario.server);
        tick(&entity, scenario);
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "cross-world respawn lost Wither REMOVE"
        );
        retired(&entity, &viewer);
        tick(&entity, scenario);
        assert!(operations(&events(&viewer), uuid).is_empty());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_wither_outer_effect_batch_revalidates_delivery() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "BatchedWither").await;
        let _anchor = scenario.join(Uuid::new_v4(), "BatchAnchor").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        let (queued, receiver) = std::sync::mpsc::channel();
        let (resume, released) = std::sync::mpsc::channel();
        let owner = entity.clone();
        let server = scenario.server.clone();
        let worker = std::thread::spawn(move || {
            let batch = crate::world::guard_effects::GuardEffects::new();
            owner.tick(owner.as_ref(), &server);
            queued.send(()).unwrap();
            released
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
            drop(batch);
        });
        scenario
            .until("Wither batched ADD queued", || receiver.try_recv().is_ok())
            .await;
        assert!(operations(&events(&viewer), uuid).is_empty());
        transfer(
            scenario,
            &viewer,
            &scenario.world(&Dimension::THE_NETHER),
            SPAWN,
        )
        .await;
        tick(&entity, scenario);
        resume.send(()).unwrap();
        scenario
            .drive(
                "Wither batch worker joins",
                tokio::task::spawn_blocking(move || worker.join().unwrap()),
            )
            .await
            .unwrap();
        let ops = operations(&events(&viewer), uuid);
        assert_eq!(
            ops,
            Vec::<i32>::new(),
            "batched Wither ADD became visible after source retirement: {ops:?}"
        );
        retired(&entity, &viewer);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_wither_bar_removed_after_transfer_and_cross_world_respawn() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let overworld = scenario.world(&Dimension::OVERWORLD);
        let nether = scenario.world(&Dimension::THE_NETHER);
        let viewer = fixture
            .viewer(Uuid::new_v4(), "WitherTrip", &overworld)
            .await;
        let _anchor = scenario.join(Uuid::new_v4(), "WitherAnchor").await;
        let source = wither(&overworld);
        let destination = wither(&nether);
        let uuid = boss(&source).bossbar_uuid;
        let destination_uuid = boss(&destination).bossbar_uuid;
        let owner = Arc::downgrade(&source);
        probe(scenario, move |_| {
            assert!(
                boss(&owner.upgrade().unwrap())
                    .bossbar_players
                    .try_lock()
                    .is_ok(),
                "Wither packet callback retained recipient guard"
            );
        });
        tick(&source, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        transfer(scenario, &viewer, &nether, SPAWN).await;
        tick(&source, scenario);
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "departed connected Wither viewer must receive exactly one REMOVE"
        );
        retired(&source, &viewer);
        tick(&source, scenario);
        assert!(operations(&events(&viewer), uuid).is_empty());
        tick(&destination, scenario);
        assert_eq!(
            operations(&events(&viewer), destination_uuid),
            vec![0, 2],
            "destination Wither must ADD independently"
        );
        // The real death command and respawn packet reach World::respawn_player.
        scenario.command("kill WitherTrip");
        assert_eq!(viewer.player.living_entity.health.load(), 0.0);
        viewer.send(
            &scenario.server,
            &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
        );
        scenario
            .until("cross-world respawn commits", || {
                scenario.server.tasks.is_empty()
            })
            .await;
        assert!(Arc::ptr_eq(&viewer.player.world(), &overworld));
        assert!(viewer.player.living_entity.health.load() > 0.0);
        assert!(
            !nether
                .players
                .load()
                .iter()
                .any(|p| Arc::ptr_eq(p, &viewer.player))
        );
        viewer.loaded(&scenario.server);
        tick(&destination, scenario);
        assert_eq!(
            operations(&events(&viewer), destination_uuid),
            vec![1],
            "cross-world respawn lost Wither REMOVE"
        );
        retired(&destination, &viewer);
        tick(&destination, scenario);
        assert!(operations(&events(&viewer), destination_uuid).is_empty());
        tick(&source, scenario);
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![0, 2],
            "returning viewer must re-add once"
        );
        tick(&source, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![2]);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_wither_death_after_viewer_departure() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "WitherDeath").await;
        let _anchor = scenario.join(Uuid::new_v4(), "DeathAnchor").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        transfer(
            scenario,
            &viewer,
            &scenario.world(&Dimension::THE_NETHER),
            SPAWN,
        )
        .await;
        scenario.command(&format!("kill {}", entity.get_entity().entity_uuid));
        assert_eq!(entity.get_living_entity().unwrap().health.load(), 0.0);
        tick(&entity, scenario);
        assert_eq!(
            operations(&events(&viewer), uuid),
            vec![1],
            "Wither death cleanup lost departed viewer REMOVE"
        );
        retired(&entity, &viewer);
        tick(&entity, scenario);
        assert!(operations(&events(&viewer), uuid).is_empty());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_same_uuid_rejoin_is_a_new_recipient() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let id = Uuid::new_v4();
        let a = scenario.join(id, "WitherRejoin").await;
        let _anchor = scenario.join(Uuid::new_v4(), "RejoinAnchor").await;
        let entity = wither(&a.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&a), uuid), vec![0, 2]);
        scenario.leave(&a).await;
        let b = scenario.join(id, "WitherRejoin").await;
        assert!(!Arc::ptr_eq(&a.player, &b.player));
        // join's single real server tick may already have reconciled the owner.
        tick(&entity, scenario);
        let ops = operations(&events(&b), uuid);
        assert_eq!(
            ops.iter().filter(|op| **op == 0).count(),
            1,
            "replacement session did not receive its own Wither ADD: {ops:?}"
        );
        assert!(
            !ops.contains(&1),
            "old-session cleanup targeted replacement"
        );
        assert!(operations(&events(&a), uuid).is_empty());
        retired(&entity, &a);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1020_g03_wither_range_health_and_normal_death_controls() {
    run(async {
        let fixture = Fixture::new().await;
        let scenario = &fixture.scenario;
        let viewer = scenario.join(Uuid::new_v4(), "WitherControls").await;
        let entity = wither(&viewer.player.world());
        let uuid = boss(&entity).bossbar_uuid;
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        viewer
            .player
            .get_entity()
            .set_pos(Vector3::new(80.0, 64.0, 0.0));
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![1]);
        viewer.player.get_entity().set_pos(SPAWN);
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![0, 2]);
        scenario.command(&format!("kill {}", entity.get_entity().entity_uuid));
        tick(&entity, scenario);
        assert_eq!(operations(&events(&viewer), uuid), vec![1]);
        scenario.stop().await;
    })
    .await;
}

#[path = "bossbar_review_tests.rs"]
mod review_tests;
