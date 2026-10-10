//! Owner-review regressions for bounded drain admission and progress.
use super::*;
use crate::world::spawn_test_support::{Fixture, proto, publish};
use pumpkin_data::{Block, biome::Biome};
use pumpkin_world::world::WorldPortalExt;

fn publish_at(fixture: &Fixture, x: i32) -> pumpkin_world::level::SyncChunk {
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.x = x;
    publish(&fixture.world, terrain)
}

#[tokio::test]
async fn owner_review_index_budget_expiry_reopens_terrain() {
    let fixture = Fixture::new();
    fixture
        .world
        .level
        .chunk_lifecycles
        .set_unload_gate(Arc::new(|_| false));
    for x in 0..4 {
        let chunk = publish_at(&fixture, x);
        fixture.world.queue_chunk_unload(&chunk);
    }
    let checks = AtomicUsize::new(0);
    // Permit every pre-index admission, then exhaust the budget after constructing the index.
    fixture
        .world
        .drain_chunk_unloads_until(|| checks.fetch_add(1, Ordering::Relaxed) >= 4);
    for x in 0..4 {
        let cell = fixture.world.level.chunk_lifecycles.at(Vector2::new(x, 0));
        assert!(
            !cell.lock().unwrap().quiescing,
            "skipped position {x} remained quiescing"
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn owner_review_budget_expiry_reopens_deferred_boundary_action() {
    let fixture = Fixture::new();
    let pos = Vector2::new(0, 0);
    publish_at(&fixture, 0);
    let permit = fixture.world.level.begin_chunk_mutation(pos);
    let queue = &fixture.world.chunk_unload_requests.actions;
    queue.push(BoundaryAction {
        pos: None,
        run: Box::new(|_| {}),
    });
    queue.push(BoundaryAction {
        pos: Some(pos),
        run: Box::new(move |_| drop(permit)),
    });
    let checks = AtomicUsize::new(0);
    fixture
        .world
        .drain_chunk_unloads_until(|| checks.fetch_add(1, Ordering::Relaxed) >= 2);
    assert!(
        !fixture
            .world
            .level
            .chunk_lifecycles
            .at(pos)
            .lock()
            .unwrap()
            .quiescing
    );
    assert_eq!(queue.len(), 1);
    fixture.world.drain_chunk_unloads_until(|| false);
    fixture.finish().await;
}

#[tokio::test]
async fn owner_review_blocked_scan_prefix_does_not_starve_tail() {
    let fixture = Fixture::new();
    let portal: Arc<dyn WorldPortalExt> =
        Arc::new(crate::world::WorldPortal(fixture.world.clone()));
    fixture
        .world
        .level
        .world_portal
        .store(Arc::new(Some(portal)));
    for x in 0..=ADMISSION_BATCH_SIZE as i32 {
        let chunk = publish_at(&fixture, x);
        fixture.world.queue_chunk_unload(&chunk);
    }
    // Select the actual DashMap tail, so randomized shard order cannot hide the blocked prefix.
    let tail = *fixture
        .world
        .chunk_unload_requests
        .terrain
        .iter()
        .last()
        .unwrap()
        .key();
    fixture
        .world
        .level
        .chunk_lifecycles
        .set_unload_gate(Arc::new(move |pos| pos == tail));
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.world.level.is_chunk_loaded(&tail) {
            fixture.world.drain_chunk_unloads_until(|| false);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.world.chunk_unload_requests.terrain.len(),
        ADMISSION_BATCH_SIZE
    );
    fixture.world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
async fn owner_review_rejected_index_admission_reopens_active_writer() {
    let fixture = Fixture::new();
    let chunk = publish_at(&fixture, 0);
    let pos = Vector2::new(0, 0);
    let permit = fixture.world.level.begin_chunk_mutation(pos);
    fixture.world.queue_chunk_unload(&chunk);
    fixture.world.drain_chunk_unloads_until(|| false);
    assert!(fixture.world.level.try_chunk_mutation(pos).is_some());
    drop(permit);
    fixture.finish().await;
}

#[tokio::test]
async fn owner_review_absent_entity_cache_retires_after_entity_drop() {
    let fixture = Fixture::new();
    let before = fixture.world.level.chunk_lifecycles.state_count();
    let pig = crate::entity::r#type::from_type(
        &pumpkin_data::entity::EntityType::PIG,
        pumpkin_util::math::vector3::Vector3::new(321.5, 64.0, 321.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    pig.get_entity()
        .set_custom_data("test", "key", pumpkin_nbt::tag::NbtTag::Int(7));
    drop(pig);
    assert_eq!(fixture.world.level.chunk_lifecycles.state_count(), before);
    fixture.finish().await;
}
