#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and barriers must be valid"
)]
use super::{
    callback_tests::StatisticCallback,
    review_tests::{DamageCallback, guest_mutation},
    *,
};
use crate::{
    entity::player::statistics::CustomStatistic,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::{
            entity_damage::EntityDamageEvent, entity_damage_by_entity::EntityDamageByEntityEvent,
        },
        player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::{
    attributes::Attributes, entity::EntityPose, item::Item, item_stack::ItemStack, potion::Effect,
};
use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering::SeqCst},
        mpsc,
    },
    time::Duration,
};

struct ByEntityCallback<F>(F);
impl<F: Fn(&mut EntityDamageByEntityEvent) + Send + Sync> EventHandler<EntityDamageByEntityEvent>
    for ByEntityCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityDamageByEntityEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn reset_in_damage_callback(by_entity: bool) {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let attacker = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    victim.set_health(10.0);
    attacker
        .living_entity
        .set_attribute_base(&Attributes::ATTACK_DAMAGE, 8.0);
    attacker.last_attacked_ticks.store(100, Relaxed);
    attacker.living_entity.set_sprinting(true);
    let reset = Arc::new(AtomicBool::new(false));
    let reached = reset.clone();
    let target = victim.clone();
    let unexpected = Arc::new(AtomicBool::new(false));
    if by_entity {
        server
            .plugin_manager
            .register::<EntityDamageByEntityEvent, _>(
                Arc::new(ByEntityCallback(
                    move |_: &mut EntityDamageByEntityEvent| {
                        target.living_entity.reset_state();
                        reached.store(true, SeqCst);
                    },
                )),
                EventPriority::Normal,
                true,
            );
    } else {
        server.plugin_manager.register::<EntityDamageEvent, _>(
            Arc::new(DamageCallback(move |_: &mut EntityDamageEvent| {
                target.living_entity.reset_state();
                reached.store(true, SeqCst);
            })),
            EventPriority::Normal,
            true,
        );
        let dispatched = unexpected.clone();
        server
            .plugin_manager
            .register::<EntityDamageByEntityEvent, _>(
                Arc::new(ByEntityCallback(
                    move |_: &mut EntityDamageByEntityEvent| {
                        dispatched.store(true, SeqCst);
                    },
                )),
                EventPriority::Normal,
                true,
            );
    }
    attacker.attack(&(victim.clone() as Arc<dyn EntityBase>));
    assert!(reset.load(SeqCst));
    assert!(
        !unexpected.load(SeqCst),
        "the old hit dispatched another event after reset"
    );
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert_eq!(victim.living_entity.last_damage_taken.load(), 0.0);
    assert_eq!(victim.get_entity().velocity.load(), Vector3::default());
    assert!(!victim.get_entity().hurt_marked.load(Relaxed));
    assert_eq!(attacker.get_custom_stat(CustomStatistic::DamageDealt), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_reset_at_damage_event_aborts_old_melee() {
    reset_in_damage_callback(false);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_reset_at_by_entity_event_aborts_old_melee() {
    reset_in_damage_callback(true);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_deaths_callback_preserves_new_inventory_effects_and_pose() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    victim
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::STONE));
    let reset = Arc::new(AtomicBool::new(false));
    let reached = reset.clone();
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(StatisticCallback(
                move |event: &mut PlayerStatisticIncrementEvent| {
                    if event.statistic_id == format!("Custom:{}", CustomStatistic::Deaths as i32) {
                        let player = &event.player;
                        player.living_entity.reset_state();
                        player
                            .inventory
                            .set_slot(0, ItemStack::new(3, &Item::DIAMOND));
                        player.living_entity.add_effect(Effect {
                            effect_type: &StatusEffect::SPEED,
                            duration: 600,
                            amplifier: 0,
                            ambient: false,
                            show_particles: false,
                            show_icon: false,
                            blend: false,
                        });
                        player.get_entity().pose.store(EntityPose::Swimming);
                        player.set_custom_stat(CustomStatistic::TimeSinceDeath, 777);
                        reached.store(true, SeqCst);
                    }
                },
            )),
            EventPriority::Normal,
            true,
        );
    assert!(!victim.damage(victim.as_ref(), 30.0, DamageType::GENERIC_KILL));
    assert!(reset.load(SeqCst));
    assert_eq!(victim.inventory.get_slot(0).item, &Item::DIAMOND);
    assert_eq!(victim.inventory.get_slot(0).item_count, 3);
    assert!(victim.living_entity.has_effect(&StatusEffect::SPEED));
    assert!(victim.get_entity().pose.load() == EntityPose::Swimming);
    assert_eq!(victim.get_custom_stat(CustomStatistic::TimeSinceDeath), 777);
    assert!(victim.has_client_loaded());
    assert!(world.entities.load().is_empty());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_callback_damages_another_entity_and_reenters_victim() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let other = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), other.clone()]));
    victim.set_health(10.0);
    let completed = Arc::new(AtomicBool::new(false));
    let result = completed.clone();
    let target = victim.clone();
    let other_target = other.clone();
    server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(DamageCallback(move |event: &mut EntityDamageEvent| {
            if event.entity_id == target.entity_id() {
                let other = other_target.clone();
                result.store(
                    guest_mutation(move || {
                        assert!(other.damage(other.as_ref(), 4.0, DamageType::GENERIC));
                    }),
                    SeqCst,
                );
            } else if event.entity_id == other_target.entity_id() {
                target.heal(2.0);
            }
        })),
        EventPriority::Normal,
        true,
    );
    assert!(
        victim
            .living_entity
            .with_damage_owned(|| { victim.damage(victim.as_ref(), 6.0, DamageType::GENERIC) })
    );
    assert!(
        completed.load(SeqCst),
        "nested damage callback retained victim ownership"
    );
    assert_eq!(victim.living_entity.health.load(), 6.0);
    assert_eq!(other.living_entity.health.load(), 16.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_opposing_melee_damps_only_restored_attacker_motion() {
    use super::super::damage_transaction::test_hooks::{self, Point};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut left = TestPlayer::new(&world);
    let mut right = TestPlayer::new(&world);
    let a = left.player.clone();
    let b = right.player.clone();
    world.players.store(Arc::new(vec![a.clone(), b.clone()]));
    for player in [&a, &b] {
        player
            .living_entity
            .set_attribute_base(&Attributes::ATTACK_DAMAGE, 2.0);
        player.last_attacked_ticks.store(100, Relaxed);
        player.living_entity.set_sprinting(true);
        player.get_entity().on_ground.store(true, Relaxed);
    }
    let old_a = Vector3::new(0.8, 0.0, 0.2);
    let old_b = Vector3::new(-0.4, 0.0, -0.8);
    a.get_entity().velocity.store(old_a);
    b.get_entity().velocity.store(old_b);
    a.get_entity().pos.store(Vector3::new(0.0, 0.0, -1.0));
    let barrier = Arc::new(Barrier::new(2));
    let (started, ready) = mpsc::channel();
    let serialized = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        let (first, second, gate, result) =
            (a.clone(), b.clone(), barrier.clone(), serialized.clone());
        scope.spawn(move || {
            let victim = second.clone();
            test_hooks::install(move |point| {
                if point == Point::MotionReady {
                    gate.wait();
                    ready.recv_timeout(Duration::from_secs(5)).unwrap();
                    result.store(victim.living_entity.wait_until_damage_contended(), SeqCst);
                }
            });
            first.attack(&(second as Arc<dyn EntityBase>));
        });
        scope.spawn(|| {
            barrier.wait();
            test_hooks::install(move |point| {
                if point == Point::Damping {
                    started.send(()).unwrap();
                }
            });
            b.attack(&(a.clone() as Arc<dyn EntityBase>));
        });
    });
    assert!(
        serialized.load(SeqCst),
        "attacker damping bypassed its victim-motion owner"
    );
    assert_eq!(
        a.get_entity().velocity.load(),
        old_a.multiply(0.6, 1.0, 0.6)
    );
    assert_eq!(
        b.get_entity().velocity.load(),
        old_b.multiply(0.6, 1.0, 0.6)
    );
    for fixture in [&mut left, &mut right] {
        fixture.player.living_entity.flush_player_motion();
        let motion_packets = super::tests::packet_ids(fixture)
            .into_iter()
            .filter(|id| *id == pumpkin_data::packet::clientbound::play::SET_ENTITY_MOTION.0)
            .count();
        assert_eq!(motion_packets, 1, "completed melee motion was replayed");
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_reset_during_attacker_damping_aborts_old_weapon_effects() {
    use super::super::damage_transaction::test_hooks::{self, Point};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let attacker = TestPlayer::new(&world).player;
    let mut sword = ItemStack::new(1, &Item::IRON_SWORD);
    sword.add_enchantment(&pumpkin_data::Enchantment::FIRE_ASPECT, 1);
    attacker.inventory.set_slot(0, sword);
    attacker.last_attacked_ticks.store(100, Relaxed);
    attacker.living_entity.set_sprinting(true);
    attacker.get_entity().on_ground.store(true, Relaxed);
    let target = victim.clone();
    let reset = Arc::new(AtomicBool::new(false));
    let reached = reset.clone();
    test_hooks::install(move |point| {
        if point == Point::Damping {
            target.living_entity.reset_state();
            reached.store(true, SeqCst);
        }
    });
    attacker.attack(&(victim.clone() as Arc<dyn EntityBase>));
    assert!(reset.load(SeqCst));
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert_eq!(victim.get_entity().fire_ticks.load(Relaxed), 0);
    assert_eq!(attacker.inventory.get_slot(0).get_damage(), 0);
    crate::server::fixture_lifecycle::finish().await;
}
