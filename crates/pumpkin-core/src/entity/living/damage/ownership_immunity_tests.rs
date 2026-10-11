#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and random streams must be valid"
)]
use super::super::damage_immunity::TEST_RANDOM;
use super::*;
use crate::{
    entity::Entity,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{Enchantment, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_util::random::{RandomImpl, xoroshiro128::Xoroshiro};
use std::sync::Arc;

fn random_seed(pattern: &[bool]) -> u64 {
    (0..100_000)
        .find(|seed| {
            let mut rng = Xoroshiro::from_seed(*seed);
            pattern
                .iter()
                .all(|immune| (rng.next_f32() < 0.5) == *immune)
        })
        .unwrap()
}

fn install_immunity_pack(server: &crate::server::Server, path: &std::path::Path) {
    let pack = path.join("datapacks/immunity");
    std::fs::create_dir_all(pack.join("data/minecraft/enchantment")).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":94,"max_format":94,"description":"immunity regression"}}"#,
    )
    .unwrap();
    for name in ["unbreaking", "mending"] {
        std::fs::write(pack.join(format!("data/minecraft/enchantment/{name}.json")), r#"{"slots":["armor"],"effects":{"minecraft:damage_immunity":[{"effect":{},"requirements":{"type":"minecraft:random_chance","chance":0.5}}]}}"#).unwrap();
    }
    server
        .datapack_manager
        .load_all(path, &["file/immunity".to_owned()], &server.recipe_manager);
    assert!(
        server
            .datapack_manager
            .get_custom_registry_entry("enchantment", "minecraft:unbreaking")
            .is_some()
    );
}

fn assert_random_calls(seed: u64, calls: usize) {
    let next = TEST_RANDOM.with(|rng| rng.borrow_mut().take().unwrap().next_f32());
    let mut reference = Xoroshiro::from_seed(seed);
    for _ in 0..calls {
        reference.next_f32();
    }
    assert_eq!(
        next,
        reference.next_f32(),
        "immunity consumed the wrong number of vanilla rolls"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_immunity_stops_after_first_matching_enchantment() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    install_immunity_pack(server.as_ref(), dir.path());
    let victim = LivingEntity::new(Entity::new(
        world,
        Vector3::default(),
        &EntityType::VILLAGER,
    ));
    let mut helmet = ItemStack::new(1, &Item::IRON_HELMET);
    for enchantment in [&Enchantment::UNBREAKING, &Enchantment::MENDING] {
        helmet.add_enchantment(enchantment, 1);
    }
    victim
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, helmet);
    victim.set_absorption(4.0);
    let seed = random_seed(&[true, false]);
    TEST_RANDOM.with(|rng| *rng.borrow_mut() = Some(Xoroshiro::from_seed(seed)));
    assert!(!victim.damage(&victim, 8.0, DamageType::GENERIC));
    assert_eq!(
        (victim.health.load(), victim.absorption.load()),
        (20.0, 4.0)
    );
    assert_random_calls(seed, 1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_random_immunity_rolls_only_before_absorption() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    install_immunity_pack(server.as_ref(), dir.path());
    let victim = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::VILLAGER,
    )));
    world.entities.store(Arc::new(vec![victim.clone()]));
    let mut helmet = ItemStack::new(1, &Item::IRON_HELMET);
    helmet.add_enchantment(&Enchantment::UNBREAKING, 1);
    victim
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, helmet);
    // This stream rejects the eighth roll. The old code reached that roll after consuming hearts.
    // The second case accepts hurtServer but rejects actuallyHurt, preserving outer hit feedback.
    for (pattern, health, absorption) in [
        (
            &[false, false, false, false, false, false, false, true][..],
            16.0,
            0.0,
        ),
        (&[false, true][..], 20.0, 4.0),
    ] {
        victim.set_health(20.0);
        victim.set_absorption(4.0);
        victim.hurt_cooldown.store(0, Relaxed);
        let seed = random_seed(pattern);
        TEST_RANDOM.with(|rng| *rng.borrow_mut() = Some(Xoroshiro::from_seed(seed)));
        let admitted = victim.damage(victim.as_ref(), 8.0, DamageType::GENERIC);
        assert_eq!(
            (admitted, victim.health.load(), victim.absorption.load()),
            (true, health, absorption),
            "an immunity reroll consumed absorption without applying the admitted hit"
        );
        assert!(victim.entity.hurt_marked.load(Relaxed));
        assert_random_calls(seed, 2);
    }
    victim.set_health(20.0);
    victim.set_absorption(4.0);
    victim.add_effect(pumpkin_data::potion::Effect {
        effect_type: &StatusEffect::FIRE_RESISTANCE,
        duration: 600,
        amplifier: 0,
        ambient: false,
        show_particles: false,
        show_icon: false,
        blend: false,
    });
    let seed = random_seed(&[false]);
    TEST_RANDOM.with(|rng| *rng.borrow_mut() = Some(Xoroshiro::from_seed(seed)));
    assert!(!victim.damage(victim.as_ref(), 8.0, DamageType::IN_FIRE));
    assert_eq!(
        (victim.health.load(), victim.absorption.load()),
        (20.0, 4.0)
    );
    assert_random_calls(seed, 1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_player_immunity_preserves_all_four_vanilla_checkpoints() {
    use crate::{entity::EntityBase, net::java::combat_test_support::TestPlayer};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    install_immunity_pack(server.as_ref(), dir.path());
    let victim = TestPlayer::new(&world).player;
    let mut helmet = ItemStack::new(1, &Item::IRON_HELMET);
    helmet.add_enchantment(&Enchantment::UNBREAKING, 1);
    victim.inventory.set_slot(39, helmet);
    for (pattern, health, absorption) in [
        (&[false, false, false, false, true][..], 16.0, 0.0),
        (&[false, false, false, true][..], 20.0, 4.0),
    ] {
        victim.set_health(20.0);
        victim.set_absorption(4.0);
        victim.living_entity.hurt_cooldown.store(0, Relaxed);
        let seed = random_seed(pattern);
        TEST_RANDOM.with(|rng| *rng.borrow_mut() = Some(Xoroshiro::from_seed(seed)));
        let admitted = victim.damage(victim.as_ref(), 8.0, DamageType::GENERIC);
        assert_eq!(
            (
                admitted,
                victim.living_entity.health.load(),
                victim.get_absorption()
            ),
            (true, health, absorption)
        );
        assert!(victim.get_entity().hurt_marked.load(Relaxed));
        assert_random_calls(seed, 4);
    }
    crate::server::fixture_lifecycle::finish().await;
}
