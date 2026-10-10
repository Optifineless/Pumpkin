use super::*;
use crate::{
    entity::{Entity, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{
    Enchantment,
    attributes::Attributes,
    data_component_impl::{BlocksAttacksImpl, EnchantmentsImpl},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::{Difficulty, Hand};
use std::sync::{Arc, Barrier};

#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
pub(super) fn packet_ids(fixture: &mut TestPlayer) -> Vec<i32> {
    fixture
        .take_packets()
        .iter()
        .map(|data| data.as_ref().get_var_int().unwrap().0)
        .collect()
}

pub(super) fn hit(victim: &Player, damage: f32, kind: DamageType, source: &dyn EntityBase) -> bool {
    victim.damage_with_context(victim, damage, kind, None, Some(source), Some(source))
}

#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
pub(super) fn raise(victim: &Player, fraction: f32) {
    let mut shield = ItemStack::new(1, &Item::SHIELD);
    let mut blocking = shield
        .get_data_component::<BlocksAttacksImpl>()
        .unwrap()
        .clone();
    let reductions = blocking.damage_reductions.to_mut();
    for reduction in reductions {
        reduction.factor = fraction;
    }
    shield.set_data_component(blocking);
    victim.inventory.set_slot(0, shield.clone());
    victim.living_entity.set_active_hand(
        Hand::Right,
        shield.clone(),
        shield.get_max_use_time() - 5,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_excess_has_no_feedback_and_ten_ticks_admits_a_full_hit() {
    use pumpkin_data::packet::clientbound::play::{DAMAGE_EVENT, HURT_ANIMATION};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let attacker = Entity::new(world, Vector3::new(1.0, 0.0, 0.0), &EntityType::ZOMBIE);
    packet_ids(&mut fixture);
    assert!(hit(&player, 6.0, DamageType::PLAYER_ATTACK, &attacker));
    assert_eq!(player.living_entity.health.load(), 14.0);
    assert_eq!(player.living_entity.hurt_time.load(Relaxed), 10);
    let packets = packet_ids(&mut fixture);
    assert_eq!(
        packets.iter().filter(|id| **id == DAMAGE_EVENT.0).count(),
        1
    );
    assert_eq!(
        packets.iter().filter(|id| **id == HURT_ANIMATION.0).count(),
        1
    );
    assert!(!hit(&player, 6.0, DamageType::PLAYER_ATTACK, &attacker));
    assert!(hit(&player, 10.0, DamageType::PLAYER_ATTACK, &attacker));
    assert_eq!(player.living_entity.health.load(), 10.0);
    assert!(
        !packet_ids(&mut fixture)
            .iter()
            .any(|id| *id == DAMAGE_EVENT.0 || *id == HURT_ANIMATION.0)
    );
    player.living_entity.hurt_cooldown.store(10, Relaxed);
    assert!(hit(&player, 2.0, DamageType::PLAYER_ATTACK, &attacker));
    assert_eq!(player.living_entity.health.load(), 8.0);
    assert!(packet_ids(&mut fixture).contains(&DAMAGE_EVENT.0));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_player_difficulty_gamerules_and_ability_tags_are_centralized() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let zombie = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(1.0, 0.0, 0.0),
        &EntityType::ZOMBIE,
    ));
    for (difficulty, expected, accepted) in [
        (Difficulty::Peaceful, 20.0, false),
        (Difficulty::Easy, 16.0, true),
        (Difficulty::Normal, 14.0, true),
        (Difficulty::Hard, 11.0, true),
    ] {
        world.set_difficulty(difficulty);
        player.living_entity.set_health(20.0);
        player.living_entity.hurt_cooldown.store(0, Relaxed);
        assert_eq!(hit(player, 6.0, DamageType::MOB_ATTACK, &zombie), accepted);
        assert_eq!(player.living_entity.health.load(), expected);
    }
    world.set_difficulty(Difficulty::Easy);
    player.living_entity.hurt_cooldown.store(0, Relaxed);
    assert!(hit(player, 1.0, DamageType::MOB_ATTACK, &zombie));
    assert_eq!(player.living_entity.health.load(), 10.0); // Easy never increases a one-point hit.
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.fall_damage = false;
        info.game_rules.drowning_damage = false;
        info.game_rules.freeze_damage = false;
        info
    });
    for kind in [DamageType::FALL, DamageType::DROWN, DamageType::FREEZE] {
        assert!(!player.living_entity.damage(player.as_ref(), 4.0, kind));
    }
    player.set_client_loaded(false);
    assert!(!player.damage(player.as_ref(), 4.0, DamageType::GENERIC_KILL));
    player.set_client_loaded(true);
    player.abilities.lock().unwrap().invulnerable = true;
    assert!(
        !player
            .living_entity
            .damage(player.as_ref(), 4.0, DamageType::GENERIC)
    );
    player.living_entity.hurt_cooldown.store(0, Relaxed);
    assert!(
        player
            .living_entity
            .damage(player.as_ref(), 1.0, DamageType::GENERIC_KILL)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_pvp_gate_includes_projectiles_and_self_owned_fireworks() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = server(dir.path());
    Arc::get_mut(&mut server)
        .unwrap()
        .advanced_config
        .pvp
        .enabled = false;
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    for kind in [
        DamageType::PLAYER_ATTACK,
        DamageType::ARROW,
        DamageType::FIREWORKS,
    ] {
        assert!(!hit(&player, 4.0, kind, player.as_ref()));
    }
    let zombie = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::ZOMBIE));
    assert!(hit(&player, 4.0, DamageType::MOB_ATTACK, &zombie));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_blocked_hits_suppress_damage_feedback_but_partial_blocks_still_push() {
    use pumpkin_data::packet::clientbound::play::{DAMAGE_EVENT, HURT_ANIMATION};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let victim = fixture.player.clone();
    let attacker = LivingEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 0.0, 1.0),
        &EntityType::ZOMBIE,
    ));
    victim.get_entity().on_ground.store(true, Relaxed);
    for (fraction, success, health, pushed) in [(1.0, false, 20.0, false), (0.5, true, 18.0, true)]
    {
        victim.living_entity.hurt_cooldown.store(0, Relaxed);
        victim.get_entity().velocity.store(Vector3::default());
        raise(&victim, fraction);
        packet_ids(&mut fixture);
        assert_eq!(
            hit(&victim, 4.0, DamageType::PLAYER_ATTACK, &attacker),
            success
        );
        assert_eq!(victim.living_entity.health.load(), health);
        assert_eq!(victim.get_entity().velocity.load().z < 0.0, pushed);
        assert_eq!(victim.get_entity().hurt_marked.load(Relaxed), pushed);
        assert!(
            !packet_ids(&mut fixture)
                .iter()
                .any(|id| *id == DAMAGE_EVENT.0 || *id == HURT_ANIMATION.0)
        );
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_projectile_direction_lethal_knockback_and_resistance() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let victim = fixture.player;
    for (kind, damage_type) in [
        (&EntityType::FIREWORK_ROCKET, DamageType::FIREWORKS),
        (&EntityType::SPLASH_POTION, DamageType::INDIRECT_MAGIC),
        (&EntityType::LINGERING_POTION, DamageType::INDIRECT_MAGIC),
    ] {
        let explosion = Entity::new(world.clone(), Vector3::new(1.0, 0.0, 0.0), kind);
        explosion.velocity.store(Vector3::new(0.0, 0.0, 1.0));
        victim.living_entity.hurt_cooldown.store(0, Relaxed);
        victim.get_entity().on_ground.store(true, Relaxed);
        victim.get_entity().velocity.store(Vector3::default());
        assert!(hit(&victim, 1.0, damage_type, &explosion));
        assert_eq!(
            victim.get_entity().velocity.load(),
            Vector3::new(-f64::from(0.4f32), 0.4, 0.0)
        );
    }
    victim.living_entity.hurt_cooldown.store(0, Relaxed);
    let arrow = Entity::new(world, Vector3::new(1.0, 0.0, 0.0), &EntityType::ARROW);
    arrow.velocity.store(Vector3::new(0.0, 0.0, 1.0));
    let living = &victim.living_entity;
    living.set_health(1.0);
    living.set_absorption(4.0);
    living.set_attribute_base(&Attributes::KNOCKBACK_RESISTANCE, 0.5);
    living.entity.on_ground.store(true, Relaxed);
    living.entity.velocity.store(Vector3::default());
    assert!(hit(&victim, 2.0, DamageType::ARROW, &arrow));
    assert_eq!(living.health.load(), 1.0);
    assert_eq!(
        living.entity.velocity.load(),
        Vector3::new(0.0, f64::from(0.4f32) * 0.5, f64::from(0.4f32) * 0.5)
    );
    living.set_absorption(0.0);
    living.hurt_cooldown.store(0, Relaxed);
    living.entity.velocity.store(Vector3::default());
    assert!(hit(&victim, 2.0, DamageType::ARROW, &arrow));
    assert_eq!(living.health.load(), 0.0);
    assert!(living.entity.velocity.load().z > 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_frost_walker_immunity_reads_the_enchantment_requirements() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let victim = &fixture.player;
    let mut boots = ItemStack::new(1, &Item::IRON_BOOTS);
    boots.set_data_component(EnchantmentsImpl {
        enchantment: std::borrow::Cow::Owned(vec![(&Enchantment::FROST_WALKER, 1)]),
    });
    victim.inventory.set_slot(36, boots);
    assert!(!victim.damage(victim.as_ref(), 1.0, DamageType::HOT_FLOOR));
    assert!(victim.damage(victim.as_ref(), 1.0, DamageType::ON_FIRE));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_two_attackers_and_healing_conserve_health_and_absorption() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let victim = fixture.player;
    let start = Barrier::new(3);
    let mut conserved = true;
    std::thread::scope(|scope| {
        for damage in [6.0, 10.0] {
            let victim = &victim;
            let start = &start;
            scope.spawn(move || {
                for _ in 0..1000 {
                    start.wait();
                    victim.damage(victim.as_ref(), damage, DamageType::GENERIC);
                    start.wait();
                }
            });
        }
        for _ in 0..1000 {
            victim.living_entity.set_health(20.0);
            victim.living_entity.set_absorption(4.0);
            victim.living_entity.hurt_cooldown.store(0, Relaxed);
            start.wait();
            start.wait();
            conserved &= victim.living_entity.health.load() == 14.0
                && victim.living_entity.absorption.load() == 0.0
                && victim.living_entity.last_damage_taken.load() == 10.0;
        }
    });
    assert!(
        conserved,
        "Concurrent hits lost damage, absorption or cooldown history"
    );
    // Heal and an accepted hit both operate on the current health, with no clamp at max health.
    victim.living_entity.set_health(10.0);
    victim.living_entity.hurt_cooldown.store(0, Relaxed);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            victim.damage(victim.as_ref(), 6.0, DamageType::GENERIC);
        });
        scope.spawn(|| victim.heal(2.0));
    });
    assert_eq!(victim.living_entity.health.load(), 6.0);
    victim.living_entity.set_health(0.0);
    victim.heal(2.0);
    assert_eq!(victim.living_entity.health.load(), 0.0);
    check_synchronous_armor_callback();
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_melee_sends_one_combined_impulse_then_restores_player_motion() {
    use pumpkin_data::packet::clientbound::play::SET_ENTITY_MOTION;
    use pumpkin_protocol::codec::lp_vector_3d::LpVector3d;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut victim_fixture = TestPlayer::new(&world);
    let attacker_fixture = TestPlayer::new(&world);
    let victim = victim_fixture.player.clone();
    let attacker = attacker_fixture.player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    victim.get_entity().on_ground.store(true, Relaxed);
    victim.get_entity().velocity.store(Vector3::default());
    attacker
        .get_entity()
        .pos
        .store(Vector3::new(0.0, 0.0, -1.0));
    attacker.last_attacked_ticks.store(20, Relaxed);
    attacker.living_entity.set_sprinting(true);
    packet_ids(&mut victim_fixture);
    attacker.attack(&(victim.clone() as Arc<dyn EntityBase>));
    let mut impulses = Vec::new();
    for bytes in victim_fixture.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0 == SET_ENTITY_MOTION.0 {
            assert_eq!(data.get_var_int().unwrap().0, victim.entity_id());
            impulses.push(LpVector3d::read(&mut data).unwrap().0);
        }
    }
    assert_eq!(impulses.len(), 1);
    assert!((impulses[0].z - 0.7).abs() < 0.001);
    assert_eq!(victim.get_entity().velocity.load(), Vector3::default());
    assert!(!victim.get_entity().hurt_marked.load(Relaxed));
    assert!(!victim.get_entity().velocity_dirty.load(Relaxed));
    assert!(!attacker.living_entity.entity.is_sprinting());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_helmet_scaling_precedes_cooldown_and_damage_stats_exclude_absorption() {
    use crate::entity::player::statistics::{CustomStatistic, StatisticCategory};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let victim = &fixture.player;
    victim
        .inventory
        .set_slot(39, ItemStack::new(1, &Item::IRON_HELMET));
    victim.living_entity.set_absorption(2.0);
    assert!(victim.damage(victim.as_ref(), 8.0, DamageType::FALLING_ANVIL));
    assert_eq!(victim.living_entity.last_damage_taken.load(), 6.0);
    assert_eq!(victim.inventory.get_slot(39).get_damage(), 3);
    // Helmet scales to 6, armor 2 leaves 5.904, absorption leaves 3.904 health damage.
    assert!((victim.living_entity.health.load() - 16.096).abs() < 0.00001);
    {
        let stats = victim.stats.lock().unwrap();
        assert_eq!(
            stats.get(
                StatisticCategory::Custom,
                CustomStatistic::DamageTaken as i32
            ),
            39
        );
        assert_eq!(
            stats.get(
                StatisticCategory::Custom,
                CustomStatistic::DamageAbsorbed as i32
            ),
            20
        );
        drop(stats);
        assert!(!victim.damage(victim.as_ref(), 6.0, DamageType::FALLING_ANVIL));
    };
    crate::server::fixture_lifecycle::finish().await;
}

#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
fn check_synchronous_armor_callback() {
    use crate::{
        plugin::player::player_item_damage::PlayerItemDamageEvent,
        plugin::{BoxFuture, EventHandler, EventPriority},
        server::Server,
    };
    struct Heal(Arc<Player>);
    impl EventHandler<PlayerItemDamageEvent> for Heal {
        fn handle_blocking<'a>(
            &'a self,
            _server: &'a Arc<Server>,
            _event: &'a mut PlayerItemDamageEvent,
        ) -> BoxFuture<'a, ()> {
            self.0.heal(2.0);
            Box::pin(async {})
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    victim.living_entity.set_health(10.0);
    victim
        .inventory
        .set_slot(38, ItemStack::new(1, &Item::IRON_CHESTPLATE));
    victim
        .living_entity
        .apply_current_equipment_attribute_modifiers();
    server.plugin_manager.register::<PlayerItemDamageEvent, _>(
        Arc::new(Heal(victim.clone())),
        EventPriority::Normal,
        true,
    );
    assert!(victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    assert!(
        (victim.living_entity.health.load() - 6.72).abs() < 0.00001,
        "health after armor callback: {}",
        victim.living_entity.health.load()
    );
}
