use super::*;
use crate::server::combat_test_support::{server, world};
use pumpkin_data::Block;
use pumpkin_util::math::vector2::Vector2;
use std::sync::Mutex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "waits on wall-clock for the asynchronous portal transfer; passes alone, times out under parallel test load (same class as the vex charge test)"]
async fn projectile_ticks_refresh_water_and_advance_portal_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(2, 64, 2, Block::WATER.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let start = Vector3::new(2.5, 64.2, 2.5);
    let snowball =
        snowball::SnowballEntity::new(Entity::new(world.clone(), start, &EntityType::SNOWBALL));
    let arrow =
        arrow::ArrowEntity::new(Entity::new(world.clone(), start, &EntityType::ARROW), None);
    let trident = trident::TridentEntity::new(
        Entity::new(world.clone(), start, &EntityType::TRIDENT),
        None,
    );
    for projectile in [&snowball as &dyn EntityBase, &arrow, &trident] {
        projectile.get_entity().velocity.store(Vector3::default());
        projectile
            .get_entity()
            .portal_cooldown
            .store(5, Ordering::Relaxed);
        projectile
            .get_entity()
            .fire_ticks
            .store(10, Ordering::Relaxed);
        projectile.tick(projectile, &server);
        assert!(projectile.get_entity().is_in_water());
        assert_eq!(
            projectile
                .get_entity()
                .portal_cooldown
                .load(Ordering::Relaxed),
            4
        );
        projectile.get_entity().set_pos(Vector3::new(
            2.5,
            f64::from(world.dimension.min_y) - 65.0,
            2.5,
        ));
        projectile.tick(projectile, &server);
        assert!(projectile.get_entity().is_removed());
    }
    // Exercise actual transit through a portal from the End back to the loaded overworld.
    let end = Arc::new(crate::world::World::load(
        pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            dir.path().join("end"),
            0,
            pumpkin_data::dimension::Dimension::THE_END,
        ),
        server.level_info.clone(),
        pumpkin_data::dimension::Dimension::THE_END,
        server.block_registry.clone(),
        Arc::downgrade(&server),
    ));
    crate::server::fixture_lifecycle::track_world(&end);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(2, 64, 2, Block::END_PORTAL.default_state.id);
    end.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    server
        .worlds
        .store(Arc::new(vec![world.clone(), end.clone()]));
    let projectile = Arc::new(snowball::SnowballEntity::new(Entity::new(
        end.clone(),
        Vector3::new(2.5, 64.7, 2.5),
        &EntityType::SNOWBALL,
    )));
    projectile
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, -0.2, 0.0));
    end.entities.store(Arc::new(vec![projectile.clone()]));
    projectile.tick(projectile.as_ref(), &server);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !Arc::ptr_eq(&projectile.get_entity().world.load(), &world)
            || end
                .get_entity_by_id(projectile.get_entity().entity_id)
                .is_some()
            || world
                .get_entity_by_id(projectile.get_entity().entity_id)
                .is_none()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        projectile
            .get_entity()
            .portal_cooldown
            .load(Ordering::Relaxed)
            > 0
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn trident_reaches_contact_and_sweeps_fire_before_breeze_deflection() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(2, 64, 2, Block::FIRE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let breeze = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 64.0, 2.5),
        &EntityType::BREEZE,
    )));
    world.entities.store(Arc::new(vec![breeze.clone()]));
    let start = Vector3::new(0.5, 64.2, 2.5);
    let trident =
        trident::TridentEntity::new(Entity::new(world, start, &EntityType::TRIDENT), None);
    trident.entity.velocity.store(Vector3::new(5.0, 0.0, 0.0));
    trident.tick(&trident, &server);
    assert!((trident.entity.pos.load().x - breeze.entity.bounding_box.load().min.x).abs() < 1e-6);
    let outgoing = trident.entity.velocity.load();
    assert!((outgoing.x + 2.475).abs() < 1e-6);
    assert!((outgoing.y + 0.05).abs() < 1e-9);
    assert_eq!(outgoing.z, 0.0);
    assert!(trident.entity.fire_ticks.load(Ordering::Relaxed) > 0);
    assert!(!trident.in_ground.load(Ordering::Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn llama_spit_moves_with_original_velocity_and_continues_after_entity_hit() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.5, 64.0, 2.5),
        &EntityType::LLAMA,
    )));
    let target = Arc::new(super::test_support::Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::new(2.5, 64.0, 2.5),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: true,
    });
    world
        .entities
        .store(Arc::new(vec![owner.clone(), target.clone()]));
    let start = Vector3::new(0.5, 64.5, 2.5);
    let spit = llama_spit::LlamaSpitEntity::new(Entity::new(world, start, &EntityType::LLAMA_SPIT));
    spit.thrown.projectile.set_owner(Some(&owner.entity));
    spit.thrown
        .entity
        .velocity
        .store(Vector3::new(4.0, 0.0, 0.0));
    spit.tick(&spit, &server);
    assert_eq!(target.hits.lock().unwrap().len(), 1);
    assert!(!spit.get_entity().is_removed());
    assert_eq!(spit.get_entity().pos.load(), Vector3::new(4.5, 64.5, 2.5));
    assert_eq!(
        spit.get_entity().velocity.load(),
        Vector3::new(f64::from(0.99f32) * 4.0, -0.06, 0.0)
    );
    spit.tick(&spit, &server);
    assert!(spit.get_entity().pos.load().x > 8.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn arrow_collision_resolves_owner_once_for_sixty_four_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::COW,
    )));
    let mut entities: Vec<Arc<dyn EntityBase>> = vec![owner.clone()];
    for _ in 0..64 {
        entities.push(Arc::new(LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::new(2.0, 65.0, 0.5),
            &EntityType::COW,
        ))));
    }
    world.entities.store(Arc::new(entities));
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world, Vector3::new(0.0, 64.0, 0.5), &EntityType::ARROW),
        Some(owner.entity.entity_id),
    );
    arrow.entity.velocity.store(Vector3::new(4.0, 0.0, 0.0));
    arrow.projectile.left_owner.store(true, Ordering::Relaxed);
    arrow.find_hit_entities(
        arrow.entity.pos.load(),
        arrow.entity.pos.load() + arrow.entity.velocity.load(),
    );
    assert_eq!(
        (
            arrow.projectile.owner_requests.load(Ordering::Relaxed),
            arrow.projectile.owner_lookups.load(Ordering::Relaxed),
        ),
        (1, 1)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn resolved_projectile_owner_is_cached_until_removal() {
    let dir = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        Some(owner.entity.entity_id),
    );
    for _ in 0..64 {
        assert!(arrow.projectile_owner().is_some());
    }
    assert_eq!(arrow.projectile.owner_lookups.load(Ordering::Relaxed), 1);
    owner.entity.remove();
    let replacement = Arc::new(LivingEntity::new(Entity::from_uuid(
        owner.entity.entity_uuid,
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![replacement.clone()]));
    assert_eq!(
        arrow.projectile_owner().unwrap().get_entity().entity_id,
        replacement.entity.entity_id
    );
    assert_eq!(arrow.projectile.owner_lookups.load(Ordering::Relaxed), 2);
    crate::server::fixture_lifecycle::finish().await;
}
