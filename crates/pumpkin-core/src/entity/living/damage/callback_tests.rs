#![expect(
    clippy::unwrap_used,
    reason = "Combat callback regression fixtures must be valid"
)]
use super::{review_tests::guest_mutation, *};
use crate::{
    entity::player::statistics::{CustomStatistic, StatisticCategory},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::SeqCst},
};

pub(super) struct StatisticCallback<F>(pub(super) F);
impl<F: Fn(&mut PlayerStatisticIncrementEvent) + Send + Sync>
    EventHandler<PlayerStatisticIncrementEvent> for StatisticCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_absorption_granted_by_a_callback_is_consumed_from_live_state() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    victim.set_absorption(2.0);
    let granted = AtomicBool::new(false);
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(StatisticCallback(
                move |event: &mut PlayerStatisticIncrementEvent| {
                    if event.statistic_id
                        == format!("Custom:{}", CustomStatistic::DamageAbsorbed as i32)
                        && !granted.swap(true, SeqCst)
                    {
                        let victim = event.player.clone();
                        assert!(guest_mutation(move || victim.set_absorption(4.0)));
                    }
                },
            )),
            EventPriority::Normal,
            true,
        );
    assert!(victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert_eq!(victim.get_absorption(), 0.0);
    assert_eq!(
        victim.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::DamageAbsorbed as i32
        ),
        60
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_excess_is_not_counted_twice_across_health_statistics_callbacks() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    assert!(victim.damage(victim.as_ref(), 6.0, DamageType::PLAYER_ATTACK));
    let nested = AtomicBool::new(false);
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(StatisticCallback(
                move |event: &mut PlayerStatisticIncrementEvent| {
                    if event.statistic_id
                        == format!("Custom:{}", CustomStatistic::DamageTaken as i32)
                        && !nested.swap(true, SeqCst)
                    {
                        assert_eq!(event.player.living_entity.last_damage_taken.load(), 6.0);
                        let victim = event.player.clone();
                        assert!(guest_mutation(move || {
                            assert!(victim.damage(
                                victim.as_ref(),
                                12.0,
                                DamageType::PLAYER_ATTACK
                            ));
                        }));
                    }
                },
            )),
            EventPriority::Normal,
            true,
        );
    assert!(victim.damage(victim.as_ref(), 10.0, DamageType::PLAYER_ATTACK));
    assert_eq!(victim.living_entity.health.load(), 8.0);
    assert_eq!(victim.living_entity.last_damage_taken.load(), 12.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_excess_sprint_knockback_is_sent_only_to_observers() {
    use crate::entity::EntityBase;
    use pumpkin_data::{attributes::Attributes, packet::clientbound::play::SET_ENTITY_MOTION};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let victim = fixture.player.clone();
    let mut observer = TestPlayer::new(&world);
    let attacker = observer.player.clone();
    world
        .players
        .store(Arc::new(vec![victim.clone(), attacker.clone()]));
    world
        .entity_tracker
        .get_tracked_entity(victim.entity_id())
        .unwrap()
        .seen_by
        .insert(attacker.gameprofile.id);
    assert!(victim.damage(victim.as_ref(), 6.0, DamageType::GENERIC));
    victim.get_entity().acknowledge_motion_delivery();
    victim.get_entity().velocity.store(Vector3::default());
    super::tests::packet_ids(&mut fixture);
    super::tests::packet_ids(&mut observer);
    attacker
        .living_entity
        .set_attribute_base(&Attributes::ATTACK_DAMAGE, 10.0);
    attacker.last_attacked_ticks.store(100, Relaxed);
    attacker.living_entity.set_sprinting(true);
    attacker.attack(&(victim.clone() as Arc<dyn EntityBase>));
    assert!(victim.get_entity().velocity_dirty.load(Relaxed));
    assert!(!victim.get_entity().hurt_marked.load(Relaxed));
    victim.living_entity.flush_player_motion();
    // Player.causeExtraKnockback sends to self only when syncVelocity is set.
    // ServerEntity.sendChanges sends this needsSync-only impulse to observers.
    assert_eq!(
        super::tests::packet_ids(&mut fixture)
            .iter()
            .filter(|id| **id == SET_ENTITY_MOTION.0)
            .count(),
        0
    );
    assert_eq!(
        super::tests::packet_ids(&mut observer)
            .iter()
            .filter(|id| **id == SET_ENTITY_MOTION.0)
            .count(),
        1
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_reset_discards_the_previous_life_melee_motion_and_statistics() {
    use crate::entity::Entity;
    use pumpkin_data::entity::EntityType;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    let attacker = Entity::new(world, Vector3::new(0.0, 0.0, -1.0), &EntityType::ZOMBIE);
    let attack = victim.living_entity.begin_melee();
    assert!(super::tests::hit(
        &victim,
        4.0,
        DamageType::PLAYER_ATTACK,
        &attacker
    ));
    let reset_completed = {
        let _released = super::super::damage_transaction::suspend_damage();
        let victim = victim.clone();
        guest_mutation(move || victim.living_entity.reset_state())
    };
    assert!(reset_completed);
    attack.finish_motion(&victim.living_entity);
    assert_eq!(victim.get_entity().velocity.load(), Vector3::default());
    assert!(!victim.get_entity().hurt_marked.load(Relaxed));
    assert_eq!(attack.health_damage(), 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

struct RescueCallback(Arc<crate::entity::player::Player>);
impl EventHandler<crate::plugin::entity::entity_resurrect::EntityResurrectEvent>
    for RescueCallback
{
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _event: &'a mut crate::plugin::entity::entity_resurrect::EntityResurrectEvent,
    ) -> BoxFuture<'a, ()> {
        let player = self.0.clone();
        let server = server.clone();
        assert!(guest_mutation(move || {
            player.living_entity.tick(player.as_ref(), &server);
            assert_eq!(player.living_entity.death_time.load(Relaxed), 0);
            player.set_health(7.0);
        }));
        Box::pin(async {})
    }
}
impl EventHandler<crate::plugin::entity::entity_death::EntityDeathEvent> for RescueCallback {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut crate::plugin::entity::entity_death::EntityDeathEvent,
    ) -> BoxFuture<'a, ()> {
        let player = self.0.clone();
        assert!(guest_mutation(move || player.set_health(7.0)));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_resurrection_and_death_callbacks_can_rescue_on_guest_threads() {
    use crate::plugin::entity::{
        entity_death::EntityDeathEvent, entity_resurrect::EntityResurrectEvent,
    };
    use pumpkin_data::{item::Item, item_stack::ItemStack};
    for totem in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let victim = TestPlayer::new(&world).player;
        if totem {
            victim
                .inventory
                .set_slot(0, ItemStack::new(1, &Item::TOTEM_OF_UNDYING));
            server.plugin_manager.register::<EntityResurrectEvent, _>(
                Arc::new(RescueCallback(victim.clone())),
                EventPriority::Normal,
                true,
            );
        } else {
            server.plugin_manager.register::<EntityDeathEvent, _>(
                Arc::new(RescueCallback(victim.clone())),
                EventPriority::Normal,
                true,
            );
        }
        assert!(victim.damage(victim.as_ref(), 40.0, DamageType::GENERIC));
        assert_eq!(victim.living_entity.health.load(), 7.0);
        assert!(!victim.living_entity.dead.load(Relaxed));
        if totem {
            assert_eq!(victim.inventory.held_item().item_count, 1);
        }
    }
    crate::server::fixture_lifecycle::finish().await;
}
