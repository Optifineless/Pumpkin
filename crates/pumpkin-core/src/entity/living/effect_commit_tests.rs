#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and bounded synchronization must succeed"
)]

use super::{Effect, LivingEntity, StatusEffect, test_hooks};
use crate::{
    entity::{Entity, EntityBase, Flag, living::EffectParticles, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{attributes::Attributes, entity::EntityType, tracked_data};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::vector3::Vector3;
use std::{
    sync::{Arc, atomic::Ordering::Relaxed, mpsc},
    time::Duration,
};

fn clear_at_commit(player: Arc<Player>, kind: &'static StatusEffect) {
    test_hooks::install(move || {
        let (done, cleared) = mpsc::channel();
        let runtime = tokio::runtime::Handle::current();
        let worker = std::thread::spawn(move || {
            let _runtime = runtime.enter();
            // The same removal entry point used by /effect clear.
            let removed = player.living_entity.remove_effect(kind);
            done.send(removed).unwrap();
        });
        assert!(
            cleared.recv_timeout(Duration::from_secs(5)).unwrap(),
            "clear must complete before the giver resumes publication"
        );
        drop(worker);
    });
}

fn effect(kind: &'static StatusEffect) -> Effect {
    Effect {
        effect_type: kind,
        duration: 600,
        amplifier: 0,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_review_clear_strength_cannot_orphan_damage_or_saved_modifier() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let world = world(&server, directory.path());
    let attacker = TestPlayer::new(&world);
    let victim = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        attacker.player.clone(),
        victim.player.clone(),
    ]));
    let living = &attacker.player.living_entity;
    living.set_attribute_base(&Attributes::ATTACK_DAMAGE, 8.0);

    // Pause at the old insertion/hook gap, clear on another command worker, then resume.
    clear_at_commit(attacker.player.clone(), &StatusEffect::STRENGTH);
    living.add_effect(effect(&StatusEffect::STRENGTH));
    assert!(!living.remove_effect(&StatusEffect::STRENGTH));

    let mut saved = NbtCompound::new();
    living.write_living_nbt(&mut saved);
    let saved_strength = saved.get_list("attributes").unwrap().iter().any(|tag| {
        let NbtTag::Compound(attribute) = tag else {
            return false;
        };
        attribute.get_list("modifiers").unwrap().iter().any(|tag| {
            matches!(tag, NbtTag::Compound(modifier)
                if modifier.get_string("id") == Some("minecraft:effect.strength"))
        })
    });
    let loaded = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::PLAYER));
    loaded.read_living_nbt_non_mut(&saved);
    assert!(!loaded.has_effect(&StatusEffect::STRENGTH));

    attacker.player.last_attacked_ticks.store(100, Relaxed);
    let health_before = victim.player.living_entity.health.load();
    attacker
        .player
        .attack(&(victim.player.clone() as Arc<dyn EntityBase>));
    assert_eq!(
        (
            living.has_effect(&StatusEffect::STRENGTH),
            living.get_attribute_value(&Attributes::ATTACK_DAMAGE),
            health_before - victim.player.living_entity.health.load(),
            saved_strength,
            loaded.get_attribute_value(&Attributes::ATTACK_DAMAGE),
        ),
        (false, 8.0, 8.0, false, 8.0),
        "a cleared Strength effect must not survive in combat or saved attributes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_review_clear_before_publication_keeps_effect_metadata_cleared() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let world = world(&server, directory.path());
    let player = TestPlayer::new(&world);
    let living = &player.player.living_entity;
    for kind in [
        &StatusEffect::INVISIBILITY,
        &StatusEffect::GLOWING,
        &StatusEffect::ABSORPTION,
    ] {
        clear_at_commit(player.player.clone(), kind);
        living.add_effect(effect(kind));
        assert!(!living.has_effect(kind));
        assert!(!living.entity.invisible.load(Relaxed));
        assert!(!living.entity.glowing.load(Relaxed));
        assert_eq!(
            living.entity.flags.load(Relaxed)
                & ((1 << Flag::Invisible as u8) | (1 << Flag::Glowing as u8)),
            0,
        );
        assert!(!living.entity.set_synced_data(
            tracked_data::entity::DATA_SHARED_FLAGS_ID,
            living.entity.flags.load(Relaxed),
        ));
        assert_eq!(living.get_absorption(), 0.0);
        assert_eq!(living.get_attribute_value(&Attributes::MAX_ABSORPTION), 0.0);
        if kind == &StatusEffect::ABSORPTION {
            assert!(
                !living
                    .entity
                    .set_synced_data(tracked_data::player::DATA_PLAYER_ABSORPTION_ID, 0.0f32,)
            );
        }
        assert!(!living.entity.set_synced_data(
            tracked_data::living_entity::EFFECT_PARTICLES,
            EffectParticles(Vec::new()),
        ));
    }
}
