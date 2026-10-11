use super::review_test_support::Fixture;
use crate::entity::passive::happy_ghast::HappyGhastEntity;
use pumpkin_data::{Block, effect::StatusEffect, entity::EntityType, potion::Effect};
use pumpkin_util::math::vector2::Vector2;
use std::sync::atomic::Ordering::Relaxed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_ageable_infinite_regeneration_uses_elapsed_world_ticks() {
    let fixture = Fixture::new();
    let animals: Vec<_> = [&EntityType::COW, &EntityType::CAT, &EntityType::GOAT]
        .into_iter()
        .map(|kind| fixture.spawn(kind, 0))
        .collect();
    for animal in &animals {
        let mob = animal.get_mob().unwrap().get_mob_entity();
        mob.ticks_lived.store(48, Relaxed);
        let living = &mob.living_entity;
        living.combat_ticks.store(48, Relaxed);
        living.set_health(living.get_max_health() - 2.0);
        living.add_effect(Effect {
            effect_type: &StatusEffect::REGENERATION,
            duration: -1,
            amplifier: 0,
            ambient: false,
            show_particles: false,
            show_icon: false,
            blend: false,
        });
    }
    for restored in [0.0, 1.0] {
        fixture.tick();
        for animal in &animals {
            let living = animal.get_living_entity().unwrap();
            assert_eq!(
                living.health.load(),
                living.get_max_health() - 2.0 + restored,
                "{} regeneration must use elapsed ticks, not growth age",
                animal.get_entity().entity_type.resource_name
            );
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_happy_ghast_timeout_uses_elapsed_world_ticks() {
    let fixture = Fixture::new();
    let entity = fixture.spawn(&EntityType::HAPPY_GHAST, 0);
    let ghast = entity
        .cast_any()
        .downcast_ref::<HappyGhastEntity>()
        .unwrap();
    ghast.mob_entity.ticks_lived.store(59, Relaxed);
    ghast.set_server_still_timeout(5);
    fixture.tick();
    assert_eq!(ghast.server_still_timeout.load(Relaxed), 5);
    fixture.tick();
    assert_eq!(ghast.server_still_timeout.load(Relaxed), 4);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_ageable_freezing_uses_elapsed_world_tick_phase() {
    let fixture = Fixture::new();
    let chunk = fixture
        .world
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap()
        .clone();
    for y in 64..=65 {
        chunk.set_block_absolute_y(9, y, 8, Block::POWDER_SNOW.default_state.id);
    }
    let cow = fixture.spawn(&EntityType::COW, -1);
    let mob = cow.get_mob().unwrap().get_mob_entity();
    mob.ticks_lived.store(38, Relaxed);
    mob.living_entity.entity.set_frozen_ticks(140);
    let health = mob.living_entity.health.load();
    fixture.tick();
    assert!(cow.get_entity().is_in_powder_snow());
    assert_eq!(mob.living_entity.health.load(), health);
    fixture.tick();
    assert_eq!(mob.living_entity.health.load(), health - 1.0);
    fixture.finish().await;
}
