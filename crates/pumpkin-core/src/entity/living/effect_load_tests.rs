use super::test_support::armor_test_world;
use super::*;

fn infinite_effect(kind: &'static StatusEffect) -> Effect {
    Effect {
        effect_type: kind,
        duration: -1,
        amplifier: 0,
        ambient: true,
        show_particles: true,
        show_icon: true,
        blend: false,
    }
}

fn round_trip_effects(kinds: &[&'static StatusEffect]) -> (tempfile::TempDir, LivingEntity) {
    let temp = tempfile::tempdir().unwrap();
    let world = armor_test_world(temp.path());
    let original = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ));
    for kind in kinds {
        original.add_effect(infinite_effect(kind));
    }
    original.set_absorption(1.0);
    let mut saved = NbtCompound::new();
    original.write_living_nbt(&mut saved);
    let loaded = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::ZOMBIE));
    loaded.read_living_nbt_non_mut(&saved);
    (temp, loaded)
}

#[tokio::test]
async fn loaded_invisibility_restores_flag() {
    let (_temp, loaded) = round_trip_effects(&[&StatusEffect::INVISIBILITY]);
    assert!(loaded.has_effect(&StatusEffect::INVISIBILITY));
    assert!(loaded.entity.invisible.load(Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn loaded_glowing_restores_flag() {
    let (_temp, loaded) = round_trip_effects(&[&StatusEffect::GLOWING]);
    assert!(loaded.has_effect(&StatusEffect::GLOWING));
    assert!(loaded.entity.glowing.load(Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn loaded_visible_effect_restores_particle_metadata() {
    let (_temp, loaded) = round_trip_effects(&[&StatusEffect::SPEED]);
    let effect = infinite_effect(&StatusEffect::SPEED);
    // set returns false only when the expected canonical metadata was already present.
    assert!(!loaded.entity.set_synced_data(
        tracked_data::living_entity::EFFECT_PARTICLES,
        EffectParticles(vec![EffectParticle::from_effect(&effect)]),
    ));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn loaded_ambient_effect_restores_ambient_metadata() {
    let (_temp, loaded) = round_trip_effects(&[&StatusEffect::SPEED]);
    assert!(
        !loaded
            .entity
            .set_synced_data(tracked_data::living_entity::EFFECT_AMBIENCE_ID, true,)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn loaded_effects_preserve_saved_attributes_and_absorption() {
    let (_temp, loaded) = round_trip_effects(&[&StatusEffect::SPEED, &StatusEffect::ABSORPTION]);
    assert!(loaded.has_effect(&StatusEffect::SPEED));
    assert!(loaded.has_effect(&StatusEffect::ABSORPTION));
    {
        let speed = loaded.attributes.read().unwrap();
        let speed = &speed[&Attributes::MOVEMENT_SPEED.id];
        assert_eq!(speed.modifiers.len(), 1);
        assert!(speed.modifiers[0].permanent);
        assert!((speed.modifiers[0].amount - 0.2).abs() < 0.000_001);
        assert_eq!(loaded.get_attribute_value(&Attributes::MAX_ABSORPTION), 4.0);
        assert_eq!(loaded.absorption.load(), 1.0);
    };
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn loaded_effect_metadata_does_not_replay_attribute_modifiers() {
    let (_temp, original) = round_trip_effects(&[&StatusEffect::SPEED]);
    // Permanent attribute NBT is authoritative, even if a command changed the
    // effect's modifier independently of the saved active-effect amplifier.
    original.update_attribute(&Attributes::MOVEMENT_SPEED, |speed| {
        let mut modifier = speed.modifiers[0].clone();
        modifier.amount = 0.125;
        speed.add_or_replace_modifier(modifier);
    });
    let mut saved = NbtCompound::new();
    original.write_living_nbt(&mut saved);
    let loaded = LivingEntity::new(Entity::new(
        original.entity.world.load_full(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ));
    loaded.read_living_nbt_non_mut(&saved);
    assert!(loaded.has_effect(&StatusEffect::SPEED));
    {
        let attributes = loaded.attributes.read().unwrap();
        let speed = &attributes[&Attributes::MOVEMENT_SPEED.id];
        assert_eq!(speed.modifiers.len(), 1);
        assert_eq!(speed.modifiers[0].amount, 0.125);
    };
    crate::server::fixture_lifecycle::finish().await;
}
