#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real last-unwatch retirement regressions require bounded storage setup"
)]

use super::b1020d_g06_test_support::{
    CHUNK, DEADLINE, Fixture, Gate, NAME, Recorder, Stage, Worker, boat, packets,
};
use crate::entity::{EntityBase, RemovalReason};
use pumpkin_data::{damage::DamageType, entity::EntityType};
use pumpkin_util::text::TextComponent;
use std::sync::{Arc, atomic::Ordering};

pub(super) async fn retire_paused(stage: Stage) {
    let mut fixture = Fixture::new();
    let storage = tokio::time::timeout(DEADLINE, fixture.world.level.get_entity_chunk(CHUNK))
        .await
        .unwrap()
        .unwrap();
    assert!(fixture.world.make_chunk_entities_live(&storage, None));
    tokio::time::timeout(
        DEADLINE,
        fixture.world.level.mark_chunks_as_newly_watched(&[CHUNK]),
    )
    .await
    .unwrap();
    let vessel = fixture.boat(&EntityType::OAK_CHEST_BOAT);
    let id = vessel.get_entity().entity_uuid;
    let observed = Recorder::register(&fixture.server);
    let gate = Gate::register(&fixture.server, stage);
    let worker = Worker::damage(vessel.clone());
    gate.reached();
    let damage = boat(vessel.as_ref()).vehicle.get_damage();
    assert!(
        tokio::time::timeout(
            DEADLINE,
            fixture.world.level.mark_chunk_as_not_watched(CHUNK)
        )
        .await
        .unwrap(),
        "actual last-unwatch must request retirement"
    );
    assert!(
        fixture
            .world
            .level
            .pending_entity_unloads()
            .iter()
            .any(|request| request.position == CHUNK)
    );
    fixture.tick(1);
    assert!(
        vessel.get_entity().removal_reason.load() == Some(RemovalReason::UnloadedToChunk),
        "world entity phase must win retirement while the actual callback is paused"
    );
    assert!(fixture.world.get_entity_by_uuid(id).is_none());
    assert!(!storage.live.load(Ordering::Acquire));
    assert_eq!(
        storage
            .data
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.get_uuid("UUID") == Some(id))
            .count(),
        1,
        "retirement must retain exactly one current saved boat identity"
    );
    assert_eq!(
        packets(&mut fixture.player, vessel.get_entity().entity_id).0,
        1
    );
    gate.resume();
    assert!(!worker.finish());
    assert_eq!(
        boat(vessel.as_ref()).vehicle.get_damage(),
        damage,
        "retired incarnation accepted stale callback damage"
    );
    assert_eq!(
        packets(&mut fixture.player, vessel.get_entity().entity_id),
        (0, 0),
        "retired incarnation published stale metadata or repeated cleanup"
    );
    assert_eq!(
        observed.destroy_count(),
        usize::from(matches!(stage, Stage::Destroy)),
        "unload winner must prevent later destruction callbacks"
    );
    assert!(
        fixture.drops().is_empty(),
        "unload winner must prevent the retired incarnation's vehicle drop"
    );
    reload_and_save(&fixture, &vessel).await;
    fixture.finish().await;
}

async fn reload_and_save(fixture: &Fixture, vessel: &Arc<dyn EntityBase>) {
    let id = vessel.get_entity().entity_uuid;
    tokio::time::timeout(
        DEADLINE,
        fixture.world.level.mark_chunks_as_newly_watched(&[CHUNK]),
    )
    .await
    .unwrap();
    let storage = tokio::time::timeout(DEADLINE, fixture.world.level.get_entity_chunk(CHUNK))
        .await
        .unwrap()
        .unwrap();
    assert!(fixture.world.make_chunk_entities_live(&storage, None));
    let restored = fixture
        .world
        .get_entity_by_uuid(id)
        .expect("normal chunk activation must restore the retired boat");
    assert!(
        !std::ptr::eq(restored.get_entity(), vessel.get_entity()),
        "retirement must never resurrect its old instance"
    );
    assert!(restored.get_entity().is_alive());
    assert_eq!(
        restored.get_entity().entity_type,
        &EntityType::OAK_CHEST_BOAT
    );
    assert_eq!(
        *restored.get_entity().custom_name.load_full(),
        Some(TextComponent::text(NAME))
    );
    assert_eq!(
        fixture
            .world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_uuid == id)
            .count(),
        1
    );
    assert!(vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC));
    assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::UnloadedToChunk));
    assert!(fixture.drops().is_empty());
    assert!(Arc::ptr_eq(
        &fixture.world.get_entity_by_uuid(id).unwrap(),
        &restored
    ));
    assert!(
        restored.damage(restored.as_ref(), 1.0, DamageType::GENERIC),
        "fresh same-life damage is a positive control after reload"
    );
    tokio::time::timeout(DEADLINE, fixture.world.save())
        .await
        .unwrap();
    assert!(storage.live.load(Ordering::Acquire));
    assert_eq!(
        storage
            .data
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.get_uuid("UUID") == Some(id))
            .count(),
        1,
        "save must contain one current incarnation and no stale duplicate"
    );
}
