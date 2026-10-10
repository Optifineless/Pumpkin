//! Follow-up regressions for absent writes, empty lookups and deferred cleanup.
use super::spawn_test_support::{Fixture, proto, publish};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering};

#[tokio::test]
async fn absent_shape_and_custom_data_writes_have_no_side_effects() {
    let fixture = Fixture::new();
    let world = &fixture.world;
    let pos = BlockPos::new(321, 64, 321);
    assert_eq!(
        world.set_block_state_with_limit(
            &pos,
            Block::CHEST.default_state.id,
            BlockFlags::NOTIFY_ALL,
            10
        ),
        Block::AIR.default_state.id
    );
    world.set_block_entity_custom_data(&pos, "test", "key", NbtTag::Int(7));
    world.remove_block_entity_custom_data(&pos, "test", "key");
    assert_eq!(world.level.chunk_lifecycles.state_count(), 0);
    assert!(world.block_entities.is_empty());
    assert!(world.custom_block_entity_data.is_empty());
    assert!(world.unsent_block_changes.lock().unwrap().is_empty());
    // A retained entity admission cell must not turn missing terrain into a writable chunk.
    drop(world.level.begin_chunk_mutation(pos.chunk_position()));
    world.set_block_entity_custom_data(&pos, "test", "key", NbtTag::Int(7));
    assert!(world.custom_block_entity_data.is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn air_lookup_does_not_take_the_lifecycle_mutex() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(1, 64, 1);
    let cell = fixture
        .world
        .level
        .chunk_lifecycles
        .at(pos.chunk_position());
    // The lookup must complete before this mutex is released, not just eventually.
    let completed = {
        let state = cell.lock().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let world = fixture.world.clone();
        let worker = std::thread::spawn(move || {
            sender.send(world.get_block_entity(&pos).is_none()).unwrap();
        });
        let completed = receiver.recv_timeout(std::time::Duration::from_secs(2));
        drop(state);
        worker.join().unwrap();
        completed
    };
    assert!(completed.unwrap());
    fixture.finish().await;
}

#[tokio::test]
async fn watched_entity_only_cleanup_does_not_request_retry() {
    let fixture = Fixture::new();
    let pos = Vector2::new(0, 0);
    fixture.world.level.get_entity_chunk(pos).await.unwrap();
    let cell = fixture.world.level.chunk_lifecycles.at(pos);
    cell.lock().unwrap().watchers = 1;
    fixture
        .world
        .level
        .should_unload
        .store(false, Ordering::Release);
    fixture.world.remove_entities_in_chunks([pos]).await;
    assert!(!fixture.world.level.should_unload.load(Ordering::Acquire));
    fixture.finish().await;
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "This regression polls cleanup while the tick read barrier blocks preparation."
)]
async fn watched_entity_only_boundary_cleanup_does_not_request_retry() {
    let fixture = Fixture::new();
    let pos = Vector2::new(0, 0);
    fixture.world.level.get_entity_chunk(pos).await.unwrap();
    fixture
        .world
        .level
        .should_unload
        .store(false, Ordering::Release);
    let mut cleanup = Box::pin(fixture.world.remove_entities_in_chunks([pos]));
    {
        let _ticks = fixture.world.hold_tick_chunks();
        assert!(futures::poll!(&mut cleanup).is_pending());
        fixture.world.level.update_chunk_watchers(&[pos], &[]);
    };
    fixture.world.drain_chunk_unloads();
    cleanup.await;
    assert!(!fixture.world.level.should_unload.load(Ordering::Acquire));
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
    fixture.finish().await;
}

#[tokio::test]
async fn entity_cache_refreshes_on_chunk_and_world_change() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let mob = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(1.5, 64.0, 1.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    let entity = mob.get_entity();
    entity.set_velocity(Vector3::new(0.1, 0.0, 0.0));
    assert!(entity.has_cached_chunk_admission(&fixture.world.level, Vector2::new(0, 0)));
    entity.set_pos(Vector3::new(17.5, 64.0, 1.5));
    entity.set_velocity(Vector3::new(0.2, 0.0, 0.0));
    assert!(entity.has_cached_chunk_admission(&fixture.world.level, Vector2::new(1, 0)));
    assert!(!entity.has_cached_chunk_admission(&fixture.world.level, Vector2::new(0, 0)));
    entity.set_world(other.world.clone());
    entity.set_velocity(Vector3::new(0.3, 0.0, 0.0));
    assert!(entity.has_cached_chunk_admission(&other.world.level, Vector2::new(1, 0)));
    assert!(!entity.has_cached_chunk_admission(&fixture.world.level, Vector2::new(1, 0)));
    drop(mob);
    fixture.finish().await;
    other.finish().await;
}

#[tokio::test]
async fn tick_identity_admission_follows_a_move_to_another_world() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let pos = Vector2::new(0, 0);
    other.world.level.get_entity_chunk(pos).await.unwrap();
    let mob = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(1.5, 64.0, 1.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    {
        let _ticks = fixture.world.hold_tick_chunks();
        let _tick = fixture.world.level.enter_tick_mutations();
        let _writer = mob.get_entity().try_begin_owned_mutation().unwrap();
        mob.get_entity().set_world(other.world.clone());
        other.world.entities.store(Arc::new(vec![mob]));
        let generation = other
            .world
            .level
            .chunk_lifecycles
            .at(pos)
            .lock()
            .unwrap()
            .generation;
        assert!(!other.world.snapshot_unloading_entities(pos, generation));
    };
    assert!(
        !other.world.entities.load()[0]
            .get_entity()
            .has_admitted_mutations()
    );
    fixture.finish().await;
    other.finish().await;
}
