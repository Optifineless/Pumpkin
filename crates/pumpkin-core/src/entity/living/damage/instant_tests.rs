#![expect(
    clippy::unwrap_used,
    reason = "Instant-effect regression fixtures must be valid"
)]
use crate::{
    entity::{Entity, living::LivingEntity},
    item::potion::{PotionApplicationSource, PotionContents},
    server::combat_test_support::{server, world},
};
use pumpkin_data::{effect::StatusEffect, entity::EntityType, potion::Effect};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_instant_effect_ticks_invert_healing_and_harming_on_zombies() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let zombie = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    )));
    world.entities.store(Arc::new(vec![zombie.clone()]));
    zombie.set_health(10.0);
    for (effect, expected) in [
        (&StatusEffect::INSTANT_DAMAGE, 14.0),
        (&StatusEffect::INSTANT_HEALTH, 8.0),
    ] {
        zombie.add_effect(Effect {
            effect_type: effect,
            duration: 1,
            amplifier: 0,
            ambient: false,
            show_particles: false,
            show_icon: false,
            blend: false,
        });
        assert_eq!(zombie.health.load(), expected, "{}", effect.minecraft_name);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orchestration_review_instant_potion_routes_invert_and_round_on_zombies() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let zombie = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::ZOMBIE));
    // HealOrHarmMobEffect.applyInstantaneousEffect: integer potency with scale * amount + 0.5.
    for (source, scale, healed, harmed) in [
        (PotionApplicationSource::Normal, 1.0, 14.0, 4.0),
        (PotionApplicationSource::Normal, 0.4, 12.0, 8.0),
        (PotionApplicationSource::AreaEffectCloud, 1.0, 12.0, 7.0),
        (PotionApplicationSource::Arrow, 0.125, 14.0, 4.0),
    ] {
        for (effect, expected) in [
            (&StatusEffect::INSTANT_DAMAGE, healed),
            (&StatusEffect::INSTANT_HEALTH, harmed),
        ] {
            zombie.with_damage_owned(|| {
                zombie.set_health(10.0);
                zombie.hurt_cooldown.store(0, Relaxed);
            });
            PotionContents::apply_effects_to(
                &zombie,
                vec![(effect, 1, 0, false, false, false)],
                scale,
                source,
            );
            assert_eq!(
                zombie.health.load(),
                expected,
                "{source:?} {}",
                effect.minecraft_name
            );
        }
    }
}
