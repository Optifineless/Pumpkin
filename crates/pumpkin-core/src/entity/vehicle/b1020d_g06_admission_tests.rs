#![expect(
    clippy::unwrap_used,
    reason = "Real callback regressions inspect bounded fixture results"
)]

use super::{
    b1020d_g06_retirement_tests::retire_paused,
    b1020d_g06_test_support::{
        Action, DEADLINE, Fixture, Gate, POS, Recorder, Stage, Worker, boat, drops, packets,
    },
};
use crate::entity::RemovalReason;
use pumpkin_data::{damage::DamageType, dimension::Dimension, entity::EntityType, item::Item};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

async fn nested(stage: Stage) {
    let fixture = Fixture::new();
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    let observed = Recorder::register(&fixture.server);
    let weak = Arc::downgrade(&vessel);
    let nested_result = Arc::new(AtomicBool::new(true));
    let result = nested_result.clone();
    Action::register(&fixture.server, stage, move || {
        let vessel = weak.upgrade().unwrap();
        result.store(
            vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC),
            Ordering::Relaxed,
        );
    });
    let accepted = vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC);
    assert_eq!(
        observed.destroy_count(),
        1,
        "{stage:?} reentry duplicated the destruction event sequence"
    );
    assert_eq!(
        observed.damage_count(),
        1,
        "{stage:?} nested damage entered an active logical operation"
    );
    assert!(!nested_result.load(Ordering::Relaxed));
    assert!(accepted);
    assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::Discarded));
    fixture.assert_named_drop(&Item::OAK_BOAT);
    fixture.finish().await;
}

async fn concurrent(stage: Stage) {
    let mut fixture = Fixture::new();
    let vessel = fixture.boat(&EntityType::BAMBOO_RAFT);
    let observed = Recorder::register(&fixture.server);
    let gate = Gate::register(&fixture.server, stage);
    fixture.player.take_packets();
    let first = Worker::damage(vessel.clone());
    gate.reached();
    let second = Worker::damage(vessel.clone());
    let second_result = second.finish();
    gate.resume();
    let first_result = first.finish();
    assert_eq!(
        observed.destroy_count(),
        1,
        "{stage:?} two workers admitted duplicate destruction events"
    );
    assert_eq!(
        observed.damage_count(),
        1,
        "{stage:?} concurrent damage entered a paused operation"
    );
    assert!(!second_result);
    assert!(first_result);
    fixture.assert_named_drop(&Item::BAMBOO_RAFT);
    assert_eq!(
        packets(&mut fixture.player, vessel.get_entity().entity_id).0,
        1,
        "concurrent lethal calls repeated cleanup"
    );
    fixture.finish().await;
}

async fn removed_during_callback(stage: Stage) {
    let mut fixture = Fixture::new();
    let vessel = fixture.boat(&EntityType::OAK_CHEST_BOAT);
    let observed = Recorder::register(&fixture.server);
    let gate = Gate::register(&fixture.server, stage);
    fixture.player.take_packets();
    let worker = Worker::damage(vessel.clone());
    gate.reached();
    vessel.get_entity().remove();
    assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::Discarded));
    let strength = boat(vessel.as_ref()).vehicle.get_damage();
    let hurt = boat(vessel.as_ref()).vehicle.get_hurt_time();
    assert_eq!(
        packets(&mut fixture.player, vessel.get_entity().entity_id).0,
        1
    );
    gate.resume();
    assert!(!worker.finish());
    assert_eq!(
        boat(vessel.as_ref()).vehicle.get_damage(),
        strength,
        "retired callback resumed into stale damage state"
    );
    assert_eq!(boat(vessel.as_ref()).vehicle.get_hurt_time(), hurt);
    assert_eq!(
        packets(&mut fixture.player, vessel.get_entity().entity_id),
        (0, 0),
        "callback removal must prevent later cleanup and stale metadata"
    );
    assert!(
        fixture.drops().is_empty(),
        "removed callback incarnation published a vehicle item"
    );
    assert_eq!(
        observed.destroy_count(),
        usize::from(matches!(stage, Stage::Destroy))
    );
    assert!(
        fixture
            .world
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .is_none()
    );
    fixture.finish().await;
}

async fn transferred_during_callback(stage: Stage) {
    let mut fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    let vessel = fixture.boat(&EntityType::BAMBOO_CHEST_RAFT);
    let observed = Recorder::register(&fixture.server);
    let gate = Gate::register(&fixture.server, stage);
    let worker = Worker::damage(vessel.clone());
    gate.reached();
    // Native plugins can invoke this public dimension-reference transition in either hook.
    vessel.get_entity().set_world(destination.clone());
    let strength = boat(vessel.as_ref()).vehicle.get_damage();
    fixture.player.take_packets();
    gate.resume();
    let accepted = worker.finish();
    assert!(
        vessel.get_entity().is_alive(),
        "old-world damage destroyed a dimension traveler"
    );
    assert!(Arc::ptr_eq(
        &vessel.get_entity().world.load_full(),
        &destination
    ));
    assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), strength);
    assert!(
        !accepted,
        "old-world callback must reject its damage operation"
    );
    assert_eq!(
        packets(&mut fixture.player, vessel.get_entity().entity_id),
        (0, 0)
    );
    assert!(fixture.drops().is_empty());
    assert!(
        drops(&destination).is_empty(),
        "old-world operation dropped in the new world"
    );
    assert_eq!(
        observed.destroy_count(),
        usize::from(matches!(stage, Stage::Destroy))
    );
    // Finish the plugin's registry migration, then exercise a fresh admitted operation.
    fixture.world.entities.rcu(|entities| {
        entities
            .iter()
            .filter(|current| !std::ptr::eq(current.get_entity(), vessel.get_entity()))
            .cloned()
            .collect::<Vec<_>>()
    });
    fixture
        .world
        .entity_tracker
        .remove_entity(vessel.as_ref(), &fixture.world);
    destination.add_entity_silent(vessel.clone());
    vessel.teleport(POS, None, None, destination.clone());
    assert!(vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC));
    assert_eq!(drops(&destination).len(), 1);
    assert!(
        fixture
            .world
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .is_none()
    );
    assert!(
        destination
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .is_none()
    );
    tokio::time::timeout(DEADLINE, destination.save())
        .await
        .unwrap();
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_boat_destroy_reentry_and_retirement_have_one_winner() {
    for stage in [Stage::Damage, Stage::Destroy] {
        nested(stage).await;
        retire_paused(stage).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_concurrent_lethal_damage_has_one_destruction_sequence() {
    for stage in [Stage::Destroy, Stage::Damage] {
        concurrent(stage).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_callback_removal_rejects_stale_damage_effects() {
    for stage in [Stage::Damage, Stage::Destroy] {
        removed_during_callback(stage).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_callback_transfer_rejects_old_world_damage_effects() {
    for stage in [Stage::Damage, Stage::Destroy] {
        transferred_during_callback(stage).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_same_life_accepted_damage_is_a_positive_control() {
    let fixture = Fixture::new();
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    let observed = Recorder::register(&fixture.server);
    assert!(vessel.damage(vessel.as_ref(), 1.0, DamageType::GENERIC));
    assert!(vessel.damage(vessel.as_ref(), 0.5, DamageType::GENERIC));
    assert!(vessel.get_entity().is_alive());
    assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), 15.0);
    assert_eq!(boat(vessel.as_ref()).vehicle.get_hurt_dir(), 1);
    assert_eq!(observed.damage_count(), 2);
    assert_eq!(observed.destroy_count(), 0);
    fixture.tick(1);
    assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), 14.0);
    assert_eq!(boat(vessel.as_ref()).vehicle.get_hurt_time(), 9);
    assert!(fixture.drops().is_empty());
    fixture.finish().await;
}
