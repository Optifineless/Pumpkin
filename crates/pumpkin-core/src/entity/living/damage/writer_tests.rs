#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and joins must be valid"
)]
use super::*;
use crate::{
    entity::{Entity, mob::Mob, passive::iron_golem::IronGolemEntity},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_nbt::NbtCompound;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_golem_repair_reads_health_after_taking_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    let golem = IronGolemEntity::new(Entity::new(
        world,
        Vector3::default(),
        &EntityType::IRON_GOLEM,
    ));
    let living = &golem.mob_entity.living_entity;
    living.set_health(50.0);
    let owner = living.own_damage();
    let repair = golem.clone();
    let thread = std::thread::spawn(move || {
        let mut ingot = ItemStack::new(1, &Item::IRON_INGOT);
        assert!(repair.mob_interact(&player, &mut ingot));
        assert!(ingot.is_empty());
    });
    assert!(living.damage_owner.wait_until_contended());
    assert!(golem.damage(golem.as_ref(), 10.0, DamageType::GENERIC));
    assert_eq!(living.health.load(), 40.0);
    drop(owner);
    thread.join().unwrap();
    assert_eq!(living.health.load(), 65.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_live_nbt_write_waits_for_combat_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let owner = victim.living_entity.own_damage();
    let writer = victim.clone();
    let thread = std::thread::spawn(move || {
        let mut nbt = NbtCompound::new();
        nbt.put_float("Health", 7.0);
        writer.living_entity.read_living_nbt_non_mut(&nbt);
    });
    assert!(victim.living_entity.damage_owner.wait_until_contended());
    assert_eq!(victim.living_entity.health.load(), 20.0);
    drop(owner);
    thread.join().unwrap();
    assert_eq!(victim.living_entity.health.load(), 7.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_damage_command_by_sets_both_source_entities() {
    use crate::command::context::command_source::CommandSource;
    let dir = tempfile::tempdir().unwrap();
    let mut server = server(dir.path());
    Arc::get_mut(&mut server)
        .unwrap()
        .advanced_config
        .pvp
        .enabled = false;
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let victim = TestPlayer::new(&world).player;
    let attacker = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    world
        .entities
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    let mut source = CommandSource::dummy();
    source.world = Some(world.clone());
    source.server = Some(server.clone());
    let command = format!(
        "damage {} 6 minecraft:player_attack by {}",
        victim.gameprofile.id, attacker.gameprofile.id
    );
    assert!(
        server
            .command_dispatcher
            .load()
            .execute_input(&command, &source)
            .is_err()
    );
    assert_eq!(victim.living_entity.health.load(), 20.0);
    let zombie = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    )));
    world.entities.store(Arc::new(vec![
        victim.clone(),
        attacker.clone(),
        zombie.clone(),
    ]));
    world.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.difficulty = pumpkin_util::Difficulty::Hard;
        info
    });
    let command = format!(
        "damage {} 6 minecraft:mob_attack by {}",
        victim.gameprofile.id, zombie.entity.entity_uuid
    );
    assert_eq!(
        server
            .command_dispatcher
            .load()
            .execute_input(&command, &source)
            .unwrap(),
        1
    );
    assert_eq!(victim.living_entity.health.load(), 11.0);
    victim.living_entity.hurt_cooldown.store(0, Relaxed);
    // Explicit cause remains authoritative even when the direct entity is a mob.
    let command = format!(
        "damage {} 6 minecraft:mob_attack by {} from {}",
        victim.gameprofile.id, zombie.entity.entity_uuid, attacker.gameprofile.id
    );
    assert!(
        server
            .command_dispatcher
            .load()
            .execute_input(&command, &source)
            .is_err()
    );
    assert_eq!(victim.living_entity.health.load(), 11.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_poison_rechecks_its_gate_after_damage_callbacks() {
    use crate::{
        entity::effect::{MobEffect, poison::PoisonMobEffect},
        plugin::{EventPriority, entity::entity_damage::EntityDamageEvent},
    };
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    victim.set_health(1.5);
    assert!(PoisonMobEffect.apply_effect_tick(&victim.living_entity, 0));
    assert_eq!(victim.living_entity.health.load(), 0.5); // Vanilla applies 1, not health - 1.
    victim.set_health(10.0);
    victim.living_entity.hurt_cooldown.store(0, Relaxed);
    let changed = victim.clone();
    server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(super::review_tests::DamageCallback(
            move |_event: &mut EntityDamageEvent| changed.set_health(1.0),
        )),
        EventPriority::Normal,
        true,
    );
    assert!(PoisonMobEffect.apply_effect_tick(&victim.living_entity, 0));
    assert_eq!(victim.living_entity.health.load(), 1.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_arrow_owner_fallback_uses_only_the_causing_entity() {
    use crate::entity::projectile::arrow::ArrowEntity;
    let dir = tempfile::tempdir().unwrap();
    let mut server = server(dir.path());
    Arc::get_mut(&mut server)
        .unwrap()
        .advanced_config
        .pvp
        .enabled = false;
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let attacker = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    let arrow = ArrowEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::ARROW),
        Some(attacker.entity_id()),
    );
    assert!(!victim.damage_with_context(
        victim.as_ref(),
        4.0,
        DamageType::ARROW,
        None,
        None,
        Some(&arrow)
    ));
    assert!(victim.damage_with_context(
        victim.as_ref(),
        4.0,
        DamageType::ARROW,
        None,
        Some(&arrow),
        None
    ));
    assert_eq!(victim.living_entity.health.load(), 16.0);
}
