use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{Block, damage::DamageType, effect::StatusEffect};
use pumpkin_util::math::vector2::Vector2;
use std::sync::atomic::AtomicUsize;

fn pearl_impact_uses_previous_tick_position(ceiling: bool) {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let block = if ceiling {
        BlockPos::new(8, 68, 8)
    } else {
        BlockPos::new(8, 64, 8)
    };
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(
        block.0.x as usize,
        block.0.y,
        block.0.z as usize,
        Block::STONE.default_state.id,
    );
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(1.0, 64.0, 1.0),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let pearl = ender_pearl::EnderPearlEntity::new_shot(
        Entity::new(world, Vector3::default(), &EntityType::ENDER_PEARL),
        &owner.entity,
    );
    let start = if ceiling {
        Vector3::new(8.5, 64.2, 8.5)
    } else {
        Vector3::new(5.5, 64.2, 8.5)
    };
    pearl.get_entity().set_pos(start);
    pearl.get_entity().set_has_no_gravity(true);
    pearl.get_entity().velocity.store(if ceiling {
        Vector3::new(0.0, 2.0, 0.0)
    } else {
        Vector3::new(2.0, 0.0, 0.0)
    });
    pearl.tick(&pearl, &server);
    assert!(!pearl.thrown.has_hit.load(Ordering::Relaxed));
    let previous = pearl.get_entity().pos.load();
    assert_ne!(previous, start);
    pearl.tick(&pearl, &server);
    assert!(pearl.thrown.has_hit.load(Ordering::Relaxed));
    assert_eq!(owner.entity.pos.load(), previous);
    let block_box = BoundingBox::new(block.0.to_f64(), block.0.to_f64().add_raw(1.0, 1.0, 1.0));
    assert!(!owner.entity.bounding_box.load().intersects(&block_box));
}

#[tokio::test]
async fn verification_pearl_wall_impact_uses_previous_tick_position() {
    pearl_impact_uses_previous_tick_position(false);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn verification_pearl_ceiling_impact_uses_previous_tick_position() {
    pearl_impact_uses_previous_tick_position(true);
    crate::server::fixture_lifecycle::finish().await;
}

fn remaining_projectile_base_tick(kind: &'static EntityType) {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::WATER.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let player = TestPlayer::new(&world).player;
    let start = Vector3::new(8.5, 64.2, 8.5);
    let projectile: Box<dyn EntityBase> = if kind == &EntityType::SHULKER_BULLET {
        Box::new(shulker_bullet::ShulkerBulletEntity::new(
            player.get_entity(),
            -1,
            start,
            crate::entity::mob::shulker::Axis::Y,
        ))
    } else if kind == &EntityType::FIREWORK_ROCKET {
        Box::new(firework_rocket::FireworkRocketEntity::new(Entity::new(
            world.clone(),
            start,
            kind,
        )))
    } else {
        Box::new(fishing_bobber::FishingBobberEntity::new(
            Entity::new(world.clone(), start, kind),
            &player,
        ))
    };
    let entity = projectile.get_entity();
    entity.set_pos(start);
    entity.velocity.store(Vector3::default());
    entity.portal_cooldown.store(5, Ordering::Relaxed);
    entity.fire_ticks.store(10, Ordering::Relaxed);
    projectile.tick(projectile.as_ref(), &server);
    assert!(entity.is_in_water());
    assert_eq!(entity.portal_cooldown.load(Ordering::Relaxed), 4);
    assert!(entity.fire_ticks.load(Ordering::Relaxed) <= 0);
    entity.set_pos(Vector3::new(
        8.5,
        f64::from(world.dimension.min_y) - 65.0,
        8.5,
    ));
    projectile.tick(projectile.as_ref(), &server);
    assert!(entity.is_removed());
}

#[tokio::test]
async fn verification_shulker_bullet_runs_base_tick() {
    remaining_projectile_base_tick(&EntityType::SHULKER_BULLET);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn verification_firework_runs_base_tick() {
    remaining_projectile_base_tick(&EntityType::FIREWORK_ROCKET);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn verification_fishing_hook_runs_base_tick() {
    remaining_projectile_base_tick(&EntityType::FISHING_BOBBER);
    crate::server::fixture_lifecycle::finish().await;
}

struct TransitingProjectile {
    snowball: snowball::SnowballEntity,
    destination: Arc<crate::world::World>,
    hits: AtomicUsize,
}

impl EntityBase for TransitingProjectile {
    fn get_entity(&self) -> &Entity {
        self.snowball.get_entity()
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn projectile_state(&self) -> Option<&ownership::ProjectileState> {
        self.snowball.projectile_state()
    }
    fn on_hit(&self, _hit: ProjectileHit) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }
    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        _amount: f32,
        kind: DamageType,
        _position: Option<Vector3<f64>>,
        _direct: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) -> bool {
        assert_eq!(kind, DamageType::ON_FIRE);
        // Force transit inside the real base tick, avoiding the asynchronous portal worker race.
        self.teleport(
            Vector3::new(8.5, 70.0, 8.5),
            None,
            None,
            self.destination.clone(),
        );
        true
    }
}

#[tokio::test]
async fn verification_projectile_skips_stale_hit_after_base_tick_dimension_change() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source = world(&server, dir.path());
    let destination = Arc::new(crate::world::World::load(
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
    crate::server::fixture_lifecycle::track_world(&destination);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::STONE.default_state.id);
    source.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let projectile = Arc::new(TransitingProjectile {
        snowball: snowball::SnowballEntity::new(Entity::new(
            source.clone(),
            Vector3::new(7.5, 64.5, 8.5),
            &EntityType::SNOWBALL,
        )),
        destination: destination.clone(),
        hits: AtomicUsize::new(0),
    });
    source.entities.store(Arc::new(vec![projectile.clone()]));
    projectile
        .get_entity()
        .velocity
        .store(Vector3::new(2.0, 0.0, 0.0));
    projectile
        .get_entity()
        .fire_ticks
        .store(20, Ordering::Relaxed);
    projectile
        .snowball
        .thrown
        .process_tick(projectile.as_ref(), &server);
    assert!(Arc::ptr_eq(
        &projectile.get_entity().world.load(),
        &destination
    ));
    assert!(projectile.get_entity().is_alive());
    assert_eq!(projectile.hits.load(Ordering::Relaxed), 0);
    assert_eq!(
        projectile.get_entity().pos.load(),
        Vector3::new(8.5, 70.0, 8.5)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn verification_shift_overflow_instant_health_lowers_live_health() {
    let dir = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(dir.path());
    let target = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    ));
    let source = snowball::SnowballEntity::new(Entity::new(
        world,
        Vector3::default(),
        &EntityType::SNOWBALL,
    ));
    target.set_health(10.0);
    // HealOrHarmMobEffect.applyInstantaneousEffect: 4 << 29 is negative in Java's signed int.
    potion_effects::apply_effect(
        &target,
        (&StatusEffect::INSTANT_HEALTH, 1, 29, false, false, false),
        1.0,
        1.0,
        &source,
        None,
        true,
    );
    assert_eq!(target.health.load(), 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn verification_dense_dead_owner_projectiles_scan_once_per_tick() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let mut entities: Vec<Arc<dyn EntityBase>> = Vec::new();
    let mut arrows = Vec::new();
    for _ in 0..256 {
        let arrow = Arc::new(arrow::ArrowEntity::new(
            Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
            None,
        ));
        arrow.projectile.set_owner(Some(&owner.entity));
        entities.push(arrow.clone());
        arrows.push(arrow);
    }
    owner.entity.remove();
    world.entities.store(Arc::new(entities));
    for _ in 0..2 {
        world.level_time.lock().unwrap().tick(true);
        for arrow in &arrows {
            for _ in 0..64 {
                assert!(arrow.projectile_owner().is_none());
            }
        }
    }
    let lookups: usize = arrows
        .iter()
        .map(|arrow| arrow.projectile.owner_lookups.load(Ordering::Relaxed))
        .sum();
    assert_eq!(
        lookups, 512,
        "256 dead-owner projectiles, 64 requests each, two ticks"
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn verification_missing_owner_cache_retries_after_tick_owner_reload_and_world_changes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        None,
    );
    let uuid = uuid::Uuid::new_v4();
    arrow.projectile.set_owner_uuid(Some(uuid));
    assert!(arrow.projectile_owner().is_none());
    let owner = Arc::new(LivingEntity::new(Entity::from_uuid(
        uuid,
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    assert!(arrow.projectile_owner().is_none());
    world.level_time.lock().unwrap().tick(true);
    assert!(arrow.projectile_owner().is_some());
    arrow.projectile.set_owner_uuid(Some(uuid::Uuid::new_v4()));
    assert!(arrow.projectile_owner().is_none());
    arrow.projectile.set_owner(Some(&owner.entity));
    assert!(arrow.projectile_owner().is_some());
    owner.entity.remove();
    assert!(arrow.projectile_owner().is_none());
    let replacement = Arc::new(LivingEntity::new(Entity::from_uuid(
        uuid,
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![replacement.clone()]));
    let mut saved = pumpkin_nbt::NbtCompound::new();
    arrow.projectile.write_nbt(&mut saved);
    arrow.projectile.read_nbt(&saved);
    assert!(arrow.projectile_owner().is_some());
    replacement.entity.remove();
    assert!(arrow.projectile_owner().is_none());
    let destination =
        crate::entity::living::test_support::armor_test_world(&dir.path().join("destination"));
    let destination_owner = Arc::new(LivingEntity::new(Entity::from_uuid(
        uuid,
        destination.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    destination
        .entities
        .store(Arc::new(vec![destination_owner.clone()]));
    arrow.entity.set_world(destination);
    assert_eq!(
        arrow.projectile_owner().unwrap().get_entity().entity_id,
        destination_owner.entity.entity_id
    );
    crate::server::fixture_lifecycle::finish().await;
}
