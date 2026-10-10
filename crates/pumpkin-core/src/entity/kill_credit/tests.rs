use super::*;
use crate::entity::death_test_world::DeathTestWorld;
use pumpkin_data::dimension::Dimension;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_accepted_absorbed_hit_records_credit_on_entity_ticks() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("Attacker");
    let victim = fixture.mob(&EntityType::COW);
    let living = victim.get_living_entity().unwrap();
    victim.get_entity().has_no_gravity.store(true, Relaxed);
    for _ in 0..3 {
        living.tick(&*victim, &fixture.server);
    }
    let health = living.health.load();
    living.set_absorption(4.0);
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(&*player),
        Some(&*player)
    ));
    assert_eq!(living.health.load(), health);
    assert_eq!(
        living.get_kill_credit().unwrap().get_entity().entity_uuid,
        player.gameprofile.id
    );
    assert_eq!(living.last_attacked_time.load(Relaxed), 3);
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.day_time = -10000;
        info.game_rules.advance_time = false;
        info
    });
    for _ in 0..100 {
        living.tick(&*victim, &fixture.server);
    }
    assert_eq!(living.hurt_by.lock().unwrap().player_memory_time, 0);
    assert!(living.get_kill_credit().is_some());
    living.tick(&*victim, &fixture.server);
    assert!(living.get_kill_credit().is_none());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_wolf_hit_resolves_its_owner_across_dimensions() {
    let fixture = DeathTestWorld::new().await;
    let owner = fixture.player("Owner");
    let nether = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    fixture.world().players.store(Arc::new(Vec::new()));
    owner.get_entity().world.store(nether.clone());
    nether.players.store(Arc::new(vec![owner.clone()]));
    let wolf = fixture.mob(&EntityType::WOLF);
    let tame = wolf.get_mob().unwrap().as_tamable().unwrap();
    tame.set_tame(true);
    tame.set_owner(Some(owner.gameprofile.id));
    let victim = fixture.mob(&EntityType::COW);
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::MOB_ATTACK,
        None,
        Some(&*wolf),
        Some(&*wolf)
    ));
    let living = victim.get_living_entity().unwrap();
    assert_eq!(
        living.get_kill_credit().unwrap().get_entity().entity_uuid,
        owner.gameprofile.id
    );
    assert_eq!(living.hurt_by.lock().unwrap().player_memory_time, 100);
    tame.set_owner(None);
    living.hurt_cooldown.store(0, Relaxed);
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::MOB_ATTACK,
        None,
        Some(&*wolf),
        Some(&*wolf)
    ));
    assert_eq!(
        living.get_kill_credit().unwrap().get_entity().entity_uuid,
        wolf.get_entity().entity_uuid
    );
    assert_eq!(living.hurt_by.lock().unwrap().player_memory_time, 0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_uuid_reload_restores_cross_dimension_attacker_for_goals() {
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.mob(&EntityType::ZOMBIE);
    let victim = fixture.mob(&EntityType::COW);
    let living = victim.get_living_entity().unwrap();
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::MOB_ATTACK,
        None,
        Some(&*attacker),
        Some(&*attacker)
    ));
    let mut saved = NbtCompound::new();
    living.write_living_nbt(&mut saved);
    let nether = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    fixture.world().entities.rcu(|entities| {
        entities
            .iter()
            .filter(|entity| entity.get_entity().entity_uuid != attacker.get_entity().entity_uuid)
            .cloned()
            .collect::<Vec<_>>()
    });
    attacker.get_entity().world.store(nether.clone());
    nether.entities.store(Arc::new(vec![attacker.clone()]));
    let reloaded = fixture.mob(&EntityType::COW);
    let reloaded_living = reloaded.get_living_entity().unwrap();
    reloaded_living.read_living_nbt_non_mut(&saved);
    assert_eq!(
        reloaded_living
            .get_kill_credit()
            .unwrap()
            .get_entity()
            .entity_uuid,
        attacker.get_entity().entity_uuid
    );
    assert_eq!(
        reloaded_living.last_attacker_id.load(Relaxed),
        attacker.get_entity().entity_id
    );
    // EntityReference accepts a dead reference until baseTick clears it, but rejects removal.
    attacker.get_living_entity().unwrap().set_health(0.0);
    assert!(reloaded_living.get_kill_credit().is_some());
    attacker.get_entity().remove();
    // Model an old world snapshot still awaiting removed-entity cleanup.
    nether.entities.store(Arc::new(vec![attacker.clone()]));
    assert!(reloaded_living.get_kill_credit().is_none());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_player_reference_rejects_removed_players() {
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.player("Attacker");
    let victim = fixture.mob(&EntityType::COW);
    assert!(victim.damage_with_context(
        &*victim,
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(&*attacker),
        Some(&*attacker)
    ));
    attacker.living_entity.set_health(0.0);
    assert!(
        victim
            .get_living_entity()
            .unwrap()
            .get_kill_credit()
            .is_some()
    );
    attacker.get_entity().remove();
    assert!(
        victim
            .get_living_entity()
            .unwrap()
            .get_kill_credit()
            .is_none()
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_outgoing_melee_memory_uses_living_ticks() {
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.mob(&EntityType::ZOMBIE);
    attacker.get_entity().has_no_gravity.store(true, Relaxed);
    let living = attacker.get_living_entity().unwrap();
    living.tick(&*attacker, &fixture.server);
    attacker.get_entity().age.store(-24000, Relaxed);
    let victim = fixture.mob(&EntityType::COW);
    attacker
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .try_attack(&*attacker, &*victim);
    assert_eq!(living.last_attack_time.load(Relaxed), 1);
    assert_eq!(
        living.last_attacking_id.load(Relaxed),
        victim.get_entity().entity_id
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_outgoing_player_attack_memory_uses_living_ticks() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("Attacker");
    player.get_entity().has_no_gravity.store(true, Relaxed);
    player.living_entity.tick(&*player, &fixture.server);
    let target = fixture.mob(&EntityType::COW);
    player.attack(&target);
    assert_eq!(player.living_entity.last_attack_time.load(Relaxed), 1);
    assert_eq!(
        player.living_entity.last_attacking_id.load(Relaxed),
        target.get_entity().entity_id
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_respawn_resets_credit_attacks_and_consumed_experience() {
    let fixture = DeathTestWorld::new().await;
    let attacker = fixture.player("Attacker");
    let victim = fixture.player("Victim");
    assert!(victim.living_entity.damage_with_context(
        &*victim,
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(&*attacker),
        Some(&*attacker)
    ));
    victim.living_entity.set_last_hurt_mob(&*attacker);
    victim.living_entity.skip_drop_experience();
    victim.living_entity.reset_state();
    assert!(victim.living_entity.get_kill_credit().is_none());
    assert_eq!(victim.living_entity.last_attacker_id.load(Relaxed), 0);
    assert_eq!(victim.living_entity.last_attacking_id.load(Relaxed), 0);
    assert!(!victim.living_entity.experience_consumed.load(Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}

#[test]
fn death_credit_reload_preserves_remaining_and_elapsed_entity_ticks() {
    let mut nbt = NbtCompound::new();
    let memory = HurtByMemory {
        player: Some(Uuid::from_u128(17)),
        player_memory_time: 23,
        mob: Some(Uuid::from_u128(29)),
        mob_timestamp: 1750,
    };
    memory.write_nbt(&mut nbt, 1800);
    assert_eq!(nbt.get_int("ticks_since_last_hurt_by_mob"), Some(50));
    let loaded = HurtByMemory::read_nbt(&nbt, 0);
    assert_eq!(loaded.player, memory.player);
    assert_eq!(loaded.player_memory_time, 23);
    assert_eq!(loaded.mob, memory.mob);
    assert_eq!(loaded.mob_timestamp, -50);
}
