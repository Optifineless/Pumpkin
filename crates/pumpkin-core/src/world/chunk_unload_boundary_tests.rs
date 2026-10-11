//! Tick-boundary unload progress and absent-position admission regressions.
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{chunk::io::Dirtiable, world::WorldPortalExt};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use super::{
    WorldPortal,
    spawn_test_support::{Fixture, proto, publish},
};

async fn fixture() -> (
    Fixture,
    pumpkin_world::level::SyncChunk,
    Arc<dyn crate::entity::EntityBase>,
) {
    let fixture = Fixture::new();
    let portal: Arc<dyn WorldPortalExt> = Arc::new(WorldPortal(fixture.world.clone()));
    fixture
        .world
        .level
        .world_portal
        .store(Arc::new(Some(portal)));
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    let chunk = publish(&fixture.world, terrain);
    chunk.mark_dirty(true);
    let entities = fixture
        .world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    fixture.world.make_chunk_entities_live(&entities, None);
    let pig = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(1.5, 64.0, 1.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    assert!(fixture.world.spawn_entity(pig.clone()));
    // These preset chunks have no scheduler holders; only the tested unload paths run.
    fixture
        .world
        .level
        .shut_down_chunk_system
        .store(true, Ordering::Release);
    fixture.world.level.level_channel.notify();
    let threads = std::mem::take(&mut *fixture.world.level.thread_tracker.lock().unwrap());
    tokio::task::spawn_blocking(move || {
        for thread in threads {
            thread.join().unwrap();
        }
    })
    .await
    .unwrap();
    fixture.world.level.chunk_system_tasks.close();
    fixture.world.level.chunk_system_tasks.wait().await;
    (fixture, chunk, pig)
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "This regression deliberately blocks unload with the tick read guard."
)]
async fn scheduled_unload_completes_at_busy_tick_boundaries() {
    let (fixture, chunk, pig) = fixture().await;
    let world = &fixture.world;
    let mut tick = world.hold_tick_chunks();
    let mut busy = Some(world.level.try_chunk_mutation(Vector2::new(0, 0)).unwrap());
    assert!(!world.level.poll_chunk_unload(&chunk));
    let mut completed_after = None;
    for index in 1..=20 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(busy.take());
        drop(tick);
        world.drain_chunk_unloads();
        tick = world.hold_tick_chunks();
        if !world.level.is_chunk_loaded(&Vector2::new(0, 0)) {
            completed_after = Some(index);
            break;
        }
    }
    drop(tick);
    assert!(
        completed_after.is_some(),
        "scheduled unload missed 20 busy tick boundaries"
    );
    assert!(world.entities.load().is_empty());
    assert!(pig.get_entity().is_removed());
    let disk = fixture.reopen_storage();
    let records = disk.get_entity_chunk(Vector2::new(0, 0)).await.unwrap();
    assert_eq!(
        records.data.lock().unwrap()[0].get_uuid("UUID"),
        Some(pig.get_entity().entity_uuid)
    );
    disk.shutdown().await.unwrap();
    world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "This regression deliberately blocks unload with the tick read guard."
)]
async fn entity_only_unload_completes_at_busy_tick_boundaries() {
    let (fixture, terrain, pig) = fixture().await;
    let world = &fixture.world;
    world.level.loaded_chunks.remove(&Vector2::new(0, 0));
    drop(terrain);
    let mut tick = world.hold_tick_chunks();
    let mut busy = Some(world.level.try_chunk_mutation(Vector2::new(0, 0)).unwrap());
    let mut cleanup = Box::pin(world.remove_entities_in_chunks([Vector2::new(0, 0)]));
    let mut done = futures::poll!(&mut cleanup).is_ready();
    assert!(!done);
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(busy.take());
        drop(tick);
        world.drain_chunk_unloads();
        tick = world.hold_tick_chunks();
        done = done || futures::poll!(&mut cleanup).is_ready();
        if done {
            break;
        }
    }
    drop(cleanup);
    drop(tick);
    assert!(done, "entity cleanup missed 20 busy tick boundaries");
    assert!(pig.get_entity().is_removed());
    assert!(world.entities.load().is_empty());
    assert!(!world.level.should_unload.load(Ordering::Acquire));
    world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
async fn absent_block_entity_lookup_does_not_create_lifecycle_state() {
    let fixture = Fixture::new();
    let before = fixture.world.level.chunk_lifecycles.state_count();
    for x in 1000..2000 {
        assert!(
            fixture
                .world
                .get_block_entity(&BlockPos::new(x * 16, 64, 0))
                .is_none()
        );
    }
    assert_eq!(fixture.world.level.chunk_lifecycles.state_count(), before);
    fixture.finish().await;
}

#[tokio::test]
async fn dimension_change_admits_the_current_destination_position() {
    let source = Fixture::new();
    let destination = Fixture::new();
    let pig = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(1.5, 64.0, 1.5),
        &source.world,
        uuid::Uuid::new_v4(),
    );
    pig.get_entity().block_pos.store(BlockPos::new(17, 64, 1));
    let lifecycle = destination
        .world
        .level
        .chunk_lifecycles
        .at(Vector2::new(1, 0));
    lifecycle.lock().unwrap().set_quiescing(true);
    pig.get_entity().set_world(destination.world.clone());
    assert!(!lifecycle.lock().unwrap().quiescing);
    source.finish().await;
    destination.finish().await;
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "This regression deliberately blocks unload with the tick read guard."
)]
async fn queued_entity_cleanup_releases_admission_when_ticks_stop() {
    let (fixture, terrain, pig) = fixture().await;
    let pos = Vector2::new(0, 0);
    fixture.world.level.loaded_chunks.remove(&pos);
    drop(terrain);
    let tick = fixture.world.hold_tick_chunks();
    let mut cleanup = Box::pin(fixture.world.remove_entities_in_chunks([pos]));
    assert!(futures::poll!(&mut cleanup).is_pending());
    fixture.world.level.cancel_token.cancel();
    assert!(futures::poll!(&mut cleanup).is_ready());
    drop(cleanup);
    drop(tick);
    fixture.world.drain_chunk_unloads();
    assert_eq!(
        fixture
            .world
            .level
            .chunk_lifecycles
            .at(pos)
            .lock()
            .unwrap()
            .mutations(),
        0
    );
    assert!(!pig.get_entity().is_removed());
    assert_eq!(fixture.world.entities.load().len(), 1);
    fixture.world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "This regression polls cleanup while a tick deliberately blocks it."
)]
async fn cancelled_prepared_boundary_reply_reopens_admission() {
    let (fixture, terrain, pig) = fixture().await;
    let pos = Vector2::new(0, 0);
    fixture.world.level.loaded_chunks.remove(&pos);
    drop(terrain);
    let tick = fixture.world.hold_tick_chunks();
    let mut cleanup = Box::pin(fixture.world.remove_entities_in_chunks([pos]));
    assert!(futures::poll!(&mut cleanup).is_pending());
    drop(tick);
    fixture.world.drain_chunk_unloads();
    assert!(
        fixture
            .world
            .level
            .chunk_lifecycles
            .at(pos)
            .lock()
            .unwrap()
            .quiescing
    );
    // Drop before the future consumes the already prepared reply.
    drop(cleanup);
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
    assert!(!fixture.world.unloading_entities.contains_key(&pos));
    let retry = fixture
        .world
        .level
        .should_unload
        .swap(false, Ordering::AcqRel);
    assert!(retry);
    assert!(!pig.get_entity().is_removed());
    if retry {
        fixture.world.remove_entities_in_chunks([pos]).await;
    }
    assert!(pig.get_entity().is_removed());
    assert!(fixture.world.entities.load().is_empty());
    fixture.world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
async fn invalidated_entity_cleanup_requests_retry_and_saves_the_new_state() {
    let (fixture, terrain, pig) = fixture().await;
    let pos = Vector2::new(0, 0);
    fixture.world.level.loaded_chunks.remove(&pos);
    drop(terrain);
    let mut pause = fixture.world.level.pause_entity_storage_publication();
    let mut cleanup = Box::pin(fixture.world.remove_entities_in_chunks([pos]));
    tokio::select! {
        () = &mut cleanup => panic!("cleanup completed before publication"),
        result = tokio::time::timeout(Duration::from_secs(15), pause.wait()) => { result.unwrap().unwrap(); }
    }
    pig.get_entity().set_velocity(Vector3::new(0.75, 0.0, 0.0));
    pause.resume();
    cleanup.await;
    let retry = fixture
        .world
        .level
        .should_unload
        .swap(false, Ordering::AcqRel);
    assert!(retry);
    assert!(!pig.get_entity().is_removed());
    if retry {
        fixture.world.remove_entities_in_chunks([pos]).await;
    }
    assert!(pig.get_entity().is_removed());
    assert!(!fixture.world.level.should_unload.load(Ordering::Acquire));
    let disk = fixture.reopen_storage();
    let records = disk.get_entity_chunk(pos).await.unwrap();
    assert_eq!(
        records.data.lock().unwrap()[0].get_list("Motion").unwrap()[0],
        pumpkin_nbt::tag::NbtTag::Double(0.75)
    );
    disk.shutdown().await.unwrap();
    fixture.world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}
