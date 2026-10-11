#![expect(
    clippy::unwrap_used,
    reason = "Spear regression fixtures and packets must be valid"
)]
use super::super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_protocol::ser::NetworkReadExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_spear_enchantment_knockback_sends_then_restores() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let attacker = TestPlayer::new(&world).player;
    let mut victim = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![attacker.clone(), victim.player.clone()]));
    attacker.last_attacked_ticks.store(100, Ordering::Relaxed);
    attacker
        .get_entity()
        .on_ground
        .store(true, Ordering::Relaxed);
    attacker
        .get_entity()
        .velocity
        .store(Vector3::new(1.0, 0.0, 1.0));
    let old = Vector3::new(0.1, 0.0, 0.2);
    victim.player.get_entity().velocity.store(old);
    victim
        .player
        .get_entity()
        .on_ground
        .store(true, Ordering::Relaxed);
    let mut spear = ItemStack::new(1, &Item::IRON_SPEAR);
    spear.add_enchantment(&Enchantment::KNOCKBACK, 1);
    victim.take_packets();
    assert!(SpearItem::stab_attack(
        &attacker,
        &server,
        Hand::Right,
        &spear,
        &(victim.player.clone() as Arc<dyn EntityBase>),
        4.0,
        StabEffects {
            damage: true,
            knockback: true,
            dismount: false
        }
    ));
    assert_eq!(
        attacker.get_entity().velocity.load(),
        Vector3::new(0.36, 0.0, 0.36)
    );
    assert!(
        !victim
            .player
            .get_entity()
            .hurt_marked
            .load(Ordering::Relaxed)
    );
    victim.player.living_entity.with_damage_owned(|| {
        victim.player.get_entity().flush_player_motion_owned();
    });
    let packets = victim.take_packets();
    let motions: Vec<_> = packets
        .iter()
        .filter(|bytes| {
            let mut data = bytes.as_ref();
            data.get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SET_ENTITY_MOTION.0
        })
        .collect();
    assert_eq!(
        motions.len(),
        1,
        "needsSync must not deliver the enchantment impulse to the victim, even next tick"
    );
    // The second LivingEntity.knockback retains movement; syncVelocity is already cleared.
    assert_eq!(
        victim.player.get_entity().velocity.load(),
        Vector3::new(0.05, 0.4, 0.6)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification4_spear_first_motion_reset_aborts_enchantment_impulse() {
    use crate::entity::living::damage_transaction::test_hooks::{self, Point};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let attacker = TestPlayer::new(&world).player;
    let victim = TestPlayer::new(&world).player;
    let old = Vector3::new(0.1, 0.0, 0.2);
    victim.get_entity().velocity.store(old);
    let mut spear = ItemStack::new(1, &Item::IRON_SPEAR);
    spear.add_enchantment(&Enchantment::KNOCKBACK, 1);
    let attack = victim.living_entity.begin_melee();
    attack.finish_motion(&victim.living_entity);
    victim
        .get_entity()
        .hurt_marked
        .store(true, Ordering::Relaxed);
    let reset = victim.clone();
    test_hooks::install(move |point| {
        if point == Point::MotionReady {
            reset.living_entity.reset_state();
        }
    });
    assert!(!SpearItem::stab_knockback(
        &attacker,
        victim.as_ref(),
        &spear,
        old,
        Some(&attack),
    ));
    assert_eq!(
        victim.get_entity().velocity.load(),
        old,
        "a stale first send must abort before enchantment knockback"
    );
    crate::server::fixture_lifecycle::finish().await;
}
