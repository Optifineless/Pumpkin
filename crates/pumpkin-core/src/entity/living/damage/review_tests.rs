#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and channels must be valid"
)]
use super::{
    callback_tests::StatisticCallback,
    tests::{hit, packet_ids, raise},
    *,
};
use crate::{
    entity::{
        Entity,
        player::{
            Player,
            statistics::{CustomStatistic, StatisticCategory},
        },
    },
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::entity_damage::EntityDamageEvent,
        player::{
            player_item_damage::PlayerItemDamageEvent,
            player_statistic_increment::PlayerStatisticIncrementEvent,
        },
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::{
    attributes::Attributes, entity::EntityType, item::Item, item_stack::ItemStack, potion::Effect,
};
use std::sync::{
    Arc, Barrier, Condvar, Mutex,
    atomic::{AtomicBool, Ordering::SeqCst},
    mpsc,
};
use std::time::Duration;

struct ArmorCallback<F>(F);
impl<F: Fn(&mut PlayerItemDamageEvent) + Send + Sync> EventHandler<PlayerItemDamageEvent>
    for ArmorCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerItemDamageEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}
pub(super) struct DamageCallback<F>(pub(super) F);
impl<F: Fn(&mut EntityDamageEvent) + Send + Sync> EventHandler<EntityDamageEvent>
    for DamageCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityDamageEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn armor(player: &Player) {
    player
        .inventory
        .set_slot(38, ItemStack::new(1, &Item::IRON_CHESTPLATE));
    player
        .living_entity
        .apply_current_equipment_attribute_modifiers();
}

// Same synchronous wait/guest-thread mutation shape as Wasm's pump_blocking bridge.
// A timeout lets the broken implementation unwind rather than hanging the test process.
pub(super) fn guest_mutation(action: impl FnOnce() + Send + 'static) -> bool {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        action();
        let _ = send.send(());
    });
    receive.recv_timeout(Duration::from_secs(5)).is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_guest_heal_and_set_health_during_armor_wear() {
    for set_health in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let victim = TestPlayer::new(&world).player;
        victim.set_health(10.0);
        armor(&victim);
        let completed = Arc::new(AtomicBool::new(false));
        let recorded = completed.clone();
        server.plugin_manager.register::<PlayerItemDamageEvent, _>(
            Arc::new(ArmorCallback(move |event: &mut PlayerItemDamageEvent| {
                let victim = event.player.clone();
                recorded.store(
                    guest_mutation(move || {
                        if set_health {
                            victim.set_health(12.0);
                        } else {
                            victim.heal(2.0);
                        }
                    }),
                    SeqCst,
                );
            })),
            EventPriority::Normal,
            true,
        );
        assert!(victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
        assert!(
            completed.load(SeqCst),
            "guest mutation waited for the callback's owner"
        );
        assert!((victim.living_entity.health.load() - 6.72).abs() < 0.00001);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_two_callbacks_can_mutate_each_others_victims() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let a = TestPlayer::new(&world).player;
    let b = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![a.clone(), b.clone()]));
    for victim in [&a, &b] {
        victim.set_health(10.0);
        armor(victim);
    }
    let barrier = Arc::new(Barrier::new(2));
    let passed = Arc::new(AtomicBool::new(true));
    let (left, right, result) = (a.clone(), b.clone(), passed.clone());
    server.plugin_manager.register::<PlayerItemDamageEvent, _>(
        Arc::new(ArmorCallback(move |event: &mut PlayerItemDamageEvent| {
            let other = if event.player.entity_id() == left.entity_id() {
                right.clone()
            } else {
                left.clone()
            };
            barrier.wait();
            if !guest_mutation(move || other.heal(2.0)) {
                result.store(false, SeqCst);
            }
        })),
        EventPriority::Normal,
        true,
    );
    std::thread::scope(|scope| {
        scope.spawn(|| a.damage(a.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
        scope.spawn(|| b.damage(b.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    });
    assert!(
        passed.load(SeqCst),
        "cross-entity callbacks retained an owner"
    );
    for victim in [&a, &b] {
        assert!((victim.living_entity.health.load() - 6.72).abs() < 0.00001);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_melee_statistics_use_each_committed_attack() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let a = TestPlayer::new(&world).player;
    let b = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), a.clone(), b.clone()]));
    let barrier = Barrier::new(2);
    let allow_stronger = Arc::new((Mutex::new(false), Condvar::new()));
    let gate = allow_stronger.clone();
    let victim_id = victim.entity_id();
    server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(DamageCallback(move |event: &mut EntityDamageEvent| {
            if event.entity_id == victim_id {
                barrier.wait();
                if event.damage == 10.0 {
                    let (mutex, ready) = gate.as_ref();
                    let (_started, timeout) = ready
                        .wait_timeout_while(
                            mutex.lock().unwrap(),
                            Duration::from_secs(5),
                            |started| !*started,
                        )
                        .unwrap();
                    assert!(!timeout.timed_out(), "the weak hit must commit first");
                }
            }
        })),
        EventPriority::Normal,
        true,
    );
    let (done, finished) = mpsc::channel();
    let finished = Mutex::new(finished);
    let first = AtomicBool::new(true);
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(StatisticCallback(
                move |event: &mut PlayerStatisticIncrementEvent| {
                    if event.player.entity_id() == victim_id
                        && event.statistic_id
                            == format!("Custom:{}", CustomStatistic::DamageTaken as i32)
                        && first.swap(false, SeqCst)
                    {
                        let (mutex, ready) = allow_stronger.as_ref();
                        *mutex.lock().unwrap() = true;
                        ready.notify_one();
                        // The first attacker resumes accounting only after the second attack finishes.
                        finished
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                    }
                },
            )),
            EventPriority::Normal,
            true,
        );
    for (attacker, damage) in [(&a, 6.0), (&b, 10.0)] {
        attacker
            .living_entity
            .set_attribute_base(&Attributes::ATTACK_DAMAGE, damage);
        attacker.last_attacked_ticks.store(100, Relaxed);
    }
    let target = victim.clone() as Arc<dyn EntityBase>;
    std::thread::scope(|scope| {
        scope.spawn(|| a.attack(&target));
        scope.spawn(|| {
            b.attack(&target);
            done.send(()).unwrap();
        });
    });
    assert_eq!(victim.living_entity.health.load(), 10.0);
    let damage = |p: &Player| {
        p.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::DamageDealt as i32,
        )
    };
    assert_eq!((damage(&a), damage(&b)), (60, 40));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_tick_cannot_deliver_an_unfinished_melee_impulse() {
    use pumpkin_data::packet::clientbound::play::SET_ENTITY_MOTION;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let victim = fixture.player.clone();
    let attacker = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    attacker
        .get_entity()
        .pos
        .store(Vector3::new(0.0, 0.0, -1.0));
    victim.get_entity().on_ground.store(true, Relaxed);
    victim.get_entity().acknowledge_motion_delivery();
    let attack = victim.living_entity.begin_melee();
    packet_ids(&mut fixture);
    assert!(hit(
        &victim,
        4.0,
        DamageType::PLAYER_ATTACK,
        attacker.as_ref()
    ));
    let completed = {
        let _callback = super::super::damage_transaction::suspend_damage();
        let victim = victim.clone();
        guest_mutation(move || victim.living_entity.flush_player_motion())
    };
    assert!(completed);
    assert!(!packet_ids(&mut fixture).contains(&SET_ENTITY_MOTION.0));
    let old = attack.finish_motion(&victim.living_entity);
    victim.living_entity.knockback(0.5, 0.0, -1.0);
    attacker.send_hurt_motion(victim.as_ref(), old);
    assert_eq!(
        packet_ids(&mut fixture)
            .iter()
            .filter(|id| **id == SET_ENTITY_MOTION.0)
            .count(),
        1
    );
    assert_eq!(victim.get_entity().velocity.load(), Vector3::default());
    assert!(!victim.get_entity().hurt_marked.load(Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_excess_history_is_visible_only_after_actually_hurt() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    armor(&victim);
    assert!(victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    let seen = Arc::new(AtomicBool::new(false));
    let recorded = seen.clone();
    server.plugin_manager.register::<PlayerItemDamageEvent, _>(
        Arc::new(ArmorCallback(move |event: &mut PlayerItemDamageEvent| {
            recorded.store(
                event.player.living_entity.last_damage_taken.load() == 6.0,
                SeqCst,
            );
        })),
        EventPriority::Normal,
        true,
    );
    assert!(victim.damage(victim.as_ref(), 10.0, DamageType::PLAYER_ATTACK));
    assert!(seen.load(SeqCst));
    assert_eq!(victim.living_entity.last_damage_taken.load(), 10.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_damage_callback_revalidates_player_gates() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let changed = victim.clone();
    server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(DamageCallback(move |_event: &mut EntityDamageEvent| {
            changed.abilities.lock().unwrap().invulnerable = true;
        })),
        EventPriority::Normal,
        true,
    );
    assert!(!victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert_eq!(victim.living_entity.hurt_cooldown.load(Relaxed), 0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_reset_during_armor_invalidates_the_old_hit() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    armor(&victim);
    server.plugin_manager.register::<PlayerItemDamageEvent, _>(
        Arc::new(ArmorCallback(move |event: &mut PlayerItemDamageEvent| {
            event.player.living_entity.reset_state();
        })),
        EventPriority::Normal,
        true,
    );
    assert!(!victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert_eq!(victim.living_entity.last_damage_taken.load(), 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_high_resistance_and_statistic_upper_bound() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    for amplifier in [5, 50, 255] {
        victim.living_entity.active_effects.lock().unwrap().insert(
            &StatusEffect::RESISTANCE,
            Effect {
                effect_type: &StatusEffect::RESISTANCE,
                amplifier,
                duration: 100,
                ambient: false,
                show_particles: false,
                show_icon: false,
                blend: false,
            },
        );
        victim.living_entity.hurt_cooldown.store(0, Relaxed);
        assert!(victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
        assert_eq!(victim.living_entity.health.load(), 20.0);
    }
    let stats = || {
        victim.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::DamageResisted as i32,
        )
    };
    assert_eq!(stats(), 180);
    victim.living_entity.hurt_cooldown.store(0, Relaxed);
    assert!(victim.damage(victim.as_ref(), f32::MAX, DamageType::PLAYER_ATTACK));
    assert_eq!(stats(), 180);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_living_block_hook_precedes_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let attacker = LivingEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 0.0, 1.0),
        &EntityType::ZOMBIE,
    ));
    victim.get_entity().on_ground.store(true, Relaxed);
    raise(&victim, 0.5);
    assert!(hit(&victim, 4.0, DamageType::PLAYER_ATTACK, &attacker));
    // Base blockedByItem pushes toward the attacker; default knockback then reverses it.
    assert!((victim.get_entity().velocity.load().z + 0.15).abs() < 0.00001);
    victim.get_entity().velocity.store(Vector3::default());
    assert!(!hit(&victim, 4.0, DamageType::PLAYER_ATTACK, &attacker));
    assert_eq!(victim.get_entity().velocity.load().z, 0.5);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_hoglin_and_ravager_respond_to_full_blocks() {
    use crate::entity::mob::{hoglin::HoglinEntity, ravager::RavagerEntity};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let hoglin = HoglinEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 0.0, 1.0),
        &EntityType::HOGLIN,
    ));
    raise(&victim, 1.0);
    assert!(!hit(&victim, 4.0, DamageType::MOB_ATTACK, hoglin.as_ref()));
    assert!(victim.get_entity().velocity.load().length() >= 0.19);
    assert!(victim.get_entity().hurt_marked.load(Relaxed));
    let ravager = RavagerEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 0.0, 1.0),
        &EntityType::RAVAGER,
    ));
    victim.get_entity().velocity.store(Vector3::default());
    ravager.blocked_by_item(victim.as_ref(), false);
    assert_eq!(
        victim.get_entity().velocity.load(),
        Vector3::new(0.0, 0.2, -4.0)
    );
    ravager.blocked_by_item(victim.as_ref(), true);
    assert_eq!(ravager.blocking.stunned.load(Relaxed), 40);
    crate::server::fixture_lifecycle::finish().await;
}
