#![expect(
    clippy::unwrap_used,
    reason = "Projectile regression fixtures must be valid"
)]
use super::*;
use crate::{
    entity::{
        Entity,
        living::damage_transaction::{
            suspend_damage,
            test_hooks::{self, Point},
        },
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{Enchantment, item::Item, item_stack::ItemStack};
use std::sync::atomic::AtomicBool;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_arrow_reset_before_punch_aborts_motion_and_effects() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    let arrow = ArrowEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::SPECTRAL_ARROW),
        None,
    );
    let mut bow = ItemStack::new(1, &Item::BOW);
    bow.add_enchantment(&Enchantment::PUNCH, 2);
    *arrow.weapon.write().unwrap() = Some(bow);
    arrow.entity.velocity.store(Vector3::new(0.0, 0.0, 2.0));
    let target = victim.clone();
    let reset = Arc::new(AtomicBool::new(false));
    let reached = reset.clone();
    test_hooks::install(move |point| {
        if point == Point::ProjectileFollowup {
            let _callback = suspend_damage();
            target.living_entity.reset_state();
            reached.store(true, Ordering::SeqCst);
        }
    });
    arrow.hit_entity(&(victim.clone() as Arc<dyn EntityBase>), Vector3::default());
    assert!(reset.load(Ordering::SeqCst));
    assert!(
        arrow.entity.is_removed(),
        "an accepted arrow must still finish its impact"
    );
    assert_eq!(victim.get_entity().velocity.load(), Vector3::default());
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert!(
        !victim
            .living_entity
            .has_effect(&pumpkin_data::effect::StatusEffect::GLOWING)
    );
    crate::server::fixture_lifecycle::finish().await;
}
