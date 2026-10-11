use super::*;
use crate::world::spawn_test_support::{Fixture, proto, publish};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};

fn pig(fixture: &Fixture, x: f64) -> Arc<dyn super::super::EntityBase> {
    crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(x, 64.0, 1.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    )
}

#[tokio::test]
async fn border_oscillation_reuses_admission_cells() {
    let fixture = Fixture::new();
    let mob = pig(&fixture, 15.98);
    let entity = mob.get_entity();
    entity.set_velocity(Vector3::new(0.05, 0.0, 0.0));
    entity.set_pos(Vector3::new(16.03, 64.0, 1.5));
    entity.set_velocity(Vector3::new(-0.05, 0.0, 0.0));
    let warmed = entity.mutation_chunk.load_full().unwrap();
    for tick in 0..20 {
        entity.set_pos(Vector3::new(
            if tick % 2 == 0 { 15.98 } else { 16.03 },
            64.0,
            1.5,
        ));
        entity.set_velocity(Vector3::new(0.05, 0.0, 0.0));
        assert!(Arc::ptr_eq(
            &warmed,
            &entity.mutation_chunk.load_full().unwrap()
        ));
    }
    fixture.finish().await;
}

#[tokio::test]
async fn mount_between_admission_checks_retries_with_riding_root() {
    let fixture = Fixture::new();
    let rider = pig(&fixture, 1.5);
    let root = pig(&fixture, 33.5);
    let (paused_tx, paused_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let (admitted_tx, admitted_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let worker_rider = rider.clone();
    let worker = std::thread::spawn(move || {
        let entity = worker_rider.get_entity();
        let mut paused = false;
        let _permit = entity
            .try_begin_move_mutation_with(entity.chunk_pos.load(), || {
                if !paused {
                    paused = true;
                    paused_tx.send(()).unwrap();
                    resume_rx
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                }
            })
            .unwrap();
        admitted_tx.send(()).unwrap();
        finish_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    });
    paused_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    root.get_entity().add_passenger(root.clone(), rider.clone());
    resume_tx.send(()).unwrap();
    admitted_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let root_admitted = fixture
        .world
        .level
        .chunk_lifecycles
        .at(Vector2::new(2, 0))
        .lock()
        .unwrap()
        .mutations();
    finish_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(
        root_admitted > 0,
        "admission missed the newly mounted riding root"
    );
    root.get_entity()
        .remove_passenger_sync(rider.get_entity().entity_id);
    fixture.finish().await;
}

#[tokio::test]
async fn left_chunk_cache_releases_cell_on_unload() {
    use pumpkin_world::{chunk::io::Dirtiable, world::WorldPortalExt};
    let fixture = Fixture::new();
    let world = &fixture.world;
    let portal: Arc<dyn WorldPortalExt> = Arc::new(crate::world::WorldPortal(world.clone()));
    world.level.world_portal.store(Arc::new(Some(portal)));
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    terrain.stage = pumpkin_world::chunk_system::StagedChunkEnum::Full;
    let chunk = publish(world, terrain);
    chunk.mark_dirty(true);
    let source = Vector2::new(0, 0);
    let mob = pig(&fixture, 1.5);
    assert!(world.spawn_entity(mob.clone()));
    mob.get_entity().set_pos(Vector3::new(17.5, 64.0, 1.5));
    mob.get_entity().set_velocity(Vector3::new(0.0, 0.0, 0.0));
    assert!(world.level.chunk_lifecycles.get(source).is_some());
    let mut scheduler = pumpkin_world::chunk_system::schedule::test_hooks::UnloadTestSchedule::new(
        &world.level,
        chunk.clone(),
    )
    .unwrap();
    // No further setter or tick on this entity: unload itself must release the left cell.
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while !scheduler.poll() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!world.level.is_chunk_loaded(&source));
    // GenerationSchedule.process_unload_queue must retire after retained save and detach.
    assert!(world.level.chunk_lifecycles.get(source).is_none());
    assert!(!mob.get_entity().is_removed());
    assert!(
        mob.get_entity()
            .has_cached_chunk_admission(&world.level, Vector2::new(1, 0))
    );
    world.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

#[tokio::test]
async fn cleared_current_cache_preserves_the_neighbour_slot() {
    let fixture = Fixture::new();
    let mob = pig(&fixture, 15.98);
    let entity = mob.get_entity();
    entity.set_velocity(Vector3::new(0.05, 0.0, 0.0));
    entity.set_pos(Vector3::new(16.03, 64.0, 1.5));
    entity.set_velocity(Vector3::new(0.0, 0.0, 0.0));
    let cached = entity.mutation_chunk.load_full().unwrap();
    let neighbour = cached.previous.as_ref().unwrap().1.clone();
    let current = fixture.world.level.chunk_lifecycles.at(Vector2::new(1, 0));
    {
        let mut state = current.lock().unwrap();
        state.set_quiescing(true);
        state.set_quiescing(false);
    };
    entity.set_velocity(Vector3::new(0.05, 0.0, 0.0));
    let refreshed = entity.mutation_chunk.load_full().unwrap();
    let (pos, previous) = refreshed.previous.as_ref().unwrap();
    assert_eq!(*pos, Vector2::new(0, 0));
    assert!(Arc::ptr_eq(previous, &neighbour));
    entity.set_pos(Vector3::new(15.98, 64.0, 1.5));
    assert!(Arc::ptr_eq(
        &refreshed,
        &entity.mutation_chunk.load_full().unwrap()
    ));
    fixture.finish().await;
}

#[tokio::test]
async fn world_change_between_admission_checks_retries_in_destination() {
    let source = Fixture::new();
    let destination = Fixture::new();
    let mob = pig(&source, 1.5);
    let entity = mob.get_entity();
    let mut changed = false;
    let permit = entity
        .try_begin_move_mutation_with(Vector2::new(0, 0), || {
            if !changed {
                changed = true;
                entity.set_world(destination.world.clone());
            }
        })
        .unwrap();
    assert!(
        destination
            .world
            .level
            .chunk_lifecycles
            .at(Vector2::new(0, 0))
            .lock()
            .unwrap()
            .mutations()
            > 0,
        "the completed permit must admit the new world's cell"
    );
    drop(permit);
    source.finish().await;
    destination.finish().await;
}

#[tokio::test]
async fn unpublished_riding_links_and_rejection_update_admission_generation() {
    use crate::entity::spawn_mount::{UnpublishedRidingTree, attach_unpublished};
    let fixture = Fixture::new();
    let root = pig(&fixture, 1.5);
    let rider = pig(&fixture, 33.5);
    let old = rider.get_entity().riding_admission.load();
    let rejected = UnpublishedRidingTree::new(&root);
    attach_unpublished(&root, rider.clone());
    assert!(
        !super::super::riding_admission::RidingAdmission::is_unmounted(
            root.get_entity().riding_admission.load()
        )
    );
    assert!(
        !super::super::riding_admission::RidingAdmission::is_unmounted(
            rider.get_entity().riding_admission.load()
        )
    );
    drop(rejected);
    let current = rider.get_entity().riding_admission.load();
    assert_ne!(
        current, old,
        "a mount/dismount must not hide behind the same flag value"
    );
    assert!(super::super::riding_admission::RidingAdmission::is_unmounted(current));
    assert!(
        super::super::riding_admission::RidingAdmission::is_unmounted(
            root.get_entity().riding_admission.load()
        )
    );
    fixture.finish().await;
}

#[tokio::test]
async fn removing_one_passenger_keeps_remaining_riding_members_admitted() {
    let fixture = Fixture::new();
    let root = pig(&fixture, 1.5);
    let first = pig(&fixture, 33.5);
    let second = pig(&fixture, 49.5);
    root.get_entity().add_passenger(root.clone(), first.clone());
    root.get_entity()
        .add_passenger(root.clone(), second.clone());
    root.get_entity()
        .remove_passenger_sync(first.get_entity().entity_id);
    {
        let _permit = root.get_entity().try_begin_mutation().unwrap();
        assert!(
            fixture
                .world
                .level
                .chunk_lifecycles
                .at(Vector2::new(3, 0))
                .lock()
                .unwrap()
                .mutations()
                > 0
        );
    };
    root.get_entity()
        .remove_passenger_sync(second.get_entity().entity_id);
    fixture.finish().await;
}
