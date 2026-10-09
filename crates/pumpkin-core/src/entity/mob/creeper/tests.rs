use super::*;
use crate::entity::{ai::goal::Goal, death_test_world::DeathTestWorld};
use pumpkin_data::{damage::DamageType, item::Item};
use pumpkin_util::math::vector3::Vector3;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killed_creeper_at_fuse_twenty_five_never_explodes_and_drops_death_loot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let table = crate::data::datapack::loot_table_loader::parse_loot_table(
        r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gunpowder"}]}]}"#,
    ).unwrap();
    fixture
        .server
        .datapack_manager
        .insert_loot_table("minecraft:entities/creeper".to_owned(), Arc::new(table));
    let creeper = CreeperEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::CREEPER,
    ));
    world.add_entity_silent(creeper.clone());
    creeper.current_fuse_time.store(25, Ordering::Relaxed);
    creeper.fuse_speed.store(1, Ordering::Relaxed);
    creeper.ignited.store(true, Ordering::Relaxed);
    assert!(creeper.damage(creeper.as_ref(), 100.0, DamageType::GENERIC));
    assert_eq!(creeper.mob_entity.living_entity.health.load(), 0.0);
    assert!(
        creeper
            .mob_entity
            .living_entity
            .dead
            .load(Ordering::Relaxed)
    );
    assert!(creeper.get_entity().pose.load() == pumpkin_data::entity::EntityPose::Dying);
    for _ in 0..19 {
        creeper.mob_tick(creeper.as_ref());
    }
    // explode() removes the entity immediately, bypassing the normal death animation.
    assert!(!creeper.get_entity().is_removed());
    assert_eq!(creeper.current_fuse_time.load(Ordering::Relaxed), 25);
    let drops: u32 = world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| {
            let stack = item.get_item_stack().lock().unwrap();
            if stack.item == &Item::GUNPOWDER {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum();
    assert_eq!(drops, 1);
    creeper.mob_entity.set_no_ai(true);
    for _ in 0..20 {
        creeper
            .mob_entity
            .living_entity
            .tick(creeper.as_ref(), &fixture.server);
    }
    assert!(creeper.get_entity().is_removed());
}

#[tokio::test]
async fn swell_goal_rejects_and_defuses_a_dying_target() {
    let dir = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(dir.path());
    let creeper = CreeperEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::CREEPER,
    ));
    let target = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(1.0, 0.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    creeper.mob_entity.set_target(Some(target.clone()));
    let mut goal = CreeperIgniteGoal::new(creeper.clone());
    assert!(goal.can_start(creeper.as_ref()));
    goal.start(creeper.as_ref());
    // Keep the target in the world during its death animation.
    target.get_living_entity().unwrap().health.store(0.0);
    assert!(!goal.can_start(creeper.as_ref()));
    creeper.fuse_speed.store(1, Ordering::Relaxed);
    goal.tick(creeper.as_ref());
    assert_eq!(creeper.fuse_speed.load(Ordering::Relaxed), -1);
}

#[tokio::test]
async fn swell_goal_rejects_and_defuses_dead_target_with_positive_health() {
    let dir = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(dir.path());
    let creeper = CreeperEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::CREEPER,
    ));
    let target = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(1.0, 0.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    creeper.mob_entity.set_target(Some(target.clone()));
    let mut goal = CreeperIgniteGoal::new(creeper.clone());
    assert!(goal.can_start(creeper.as_ref()));
    goal.start(creeper.as_ref());
    let living = target.get_living_entity().unwrap();
    assert!(living.health.load() > 0.0);
    living.dead.store(true, Ordering::Relaxed);
    assert!(!goal.can_start(creeper.as_ref()));
    creeper.fuse_speed.store(1, Ordering::Relaxed);
    // SwellGoal.canUse's positive-swell shortcut survives even when the target is dead.
    assert!(goal.can_start(creeper.as_ref()));
    goal.tick(creeper.as_ref());
    assert_eq!(creeper.fuse_speed.load(Ordering::Relaxed), -1);
}

#[tokio::test]
async fn creeper_blast_damage_and_knockback_stop_at_six_or_twelve_blocks() {
    use crate::entity::living::LivingEntity;
    use crate::world::spawn_test_support::{Fixture, proto, publish};
    use pumpkin_data::{Block, biome::Biome};
    for (charged, distances, middle_damage) in [
        (false, [3.0, 6.0, 6.01], 16.75),
        (true, [6.0, 12.0, 12.01], 32.5),
    ] {
        let fixture = Fixture::new();
        let world = &fixture.world;
        for x in -1..=1 {
            for z in -1..=1 {
                let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
                terrain.x = x;
                terrain.z = z;
                publish(world, terrain);
            }
        }
        let center = Vector3::new(8.5, 100.0, 8.5);
        let creeper = CreeperEntity::new(Entity::new(world.clone(), center, &EntityType::CREEPER));
        creeper.charged.store(charged, Ordering::Relaxed);
        world.add_entity_silent(creeper.clone());
        let victims: Vec<_> = distances
            .into_iter()
            .map(|distance| {
                let living = Arc::new(LivingEntity::new(Entity::new(
                    world.clone(),
                    center.add_raw(distance, 0.0, 0.0),
                    &EntityType::COW,
                )));
                living.set_max_health(100.0);
                living.health.store(100.0);
                world.add_entity_silent(living.clone());
                living
            })
            .collect();
        creeper.explode();
        // Creeper.explodeCreeper -> ServerExplosion.hurtEntities / ExplosionDamageCalculator.
        assert_eq!(100.0 - victims[0].health.load(), middle_damage);
        assert!((victims[0].entity.velocity.load().length_squared() - 0.25).abs() < 1e-6);
        assert_eq!(victims[1].health.load(), 99.0);
        assert_eq!(victims[1].entity.velocity.load(), Vector3::default());
        assert_eq!(victims[2].health.load(), 100.0);
        assert_eq!(victims[2].entity.velocity.load(), Vector3::default());
        fixture.finish().await;
    }
}
