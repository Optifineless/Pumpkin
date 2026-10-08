use super::conditions::number_provider;
use crate::entity::{
    EntityBase,
    equipment_damage::{EquippedItem, damage_equipped_item},
};
use crate::world::{ExplosionInteraction, SimpleExplosionDamageCalculator, explosion::Explosion};
use pumpkin_data::{
    damage::DamageType,
    effect::StatusEffect,
    particle::Particle,
    potion::Effect,
    sound::{Sound, SoundCategory},
};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::{
    math::vector3::Vector3,
    random::{RandomImpl, xoroshiro128::Xoroshiro},
};
use std::sync::Arc;

// EnchantmentEntityEffect.apply receives the enchanted stack independently of its target.
pub(super) fn apply(
    effect: &NbtCompound,
    level: i32,
    owner: &dyn EntityBase,
    item: &EquippedItem,
    target: &dyn EntityBase,
    rng: &mut Xoroshiro,
) {
    let value = |key| {
        effect
            .get(key)
            .and_then(|value| number_provider(value, level))
    };
    match effect.get_string("type") {
        Some("minecraft:all_of") => {
            for child in effect.get_list("effects").into_iter().flatten() {
                if let Some(child) = child.extract_compound() {
                    apply(child, level, owner, item, target, rng);
                }
            }
        }
        Some("minecraft:damage_entity") => {
            if let Some((damage_type, damage)) = damage_entity_effect(effect, level, rng) {
                target.damage_with_context(
                    target,
                    damage,
                    damage_type,
                    None,
                    Some(owner),
                    Some(owner),
                );
            }
        }
        Some("minecraft:change_item_damage") => {
            if let Some(amount) = value("amount") {
                damage_equipped_item(owner, item, amount as i32);
            }
        }
        Some("minecraft:ignite") => {
            if let Some(seconds) = value("duration") {
                super::effects::ignite::ignite_target(target, seconds);
            }
        }
        Some("minecraft:apply_mob_effect") => apply_mob_effect(effect, level, target, rng),
        Some("minecraft:explode") => explode(effect, level, target),
        Some("minecraft:play_sound") => {
            if let (Some(sound), Some(volume), Some(pitch)) = (
                effect
                    .get_string("sound")
                    .and_then(|name| Sound::from_name(registry_path(name))),
                effect.get_numeric_float("volume"),
                effect.get_numeric_float("pitch"),
            ) {
                let entity = target.get_entity();
                if !entity.is_silent() {
                    entity.world.load().play_sound_fine(
                        sound,
                        SoundCategory::Players,
                        &entity.pos.load(),
                        volume,
                        pitch,
                    );
                }
            }
        }
        _ => tracing::warn!("Unsupported post-attack enchantment effect"),
    }
}

// ApplyMobEffect.apply uses rounded seconds and amplifier after random selection.
fn apply_mob_effect(
    effect: &NbtCompound,
    level: i32,
    target: &dyn EntityBase,
    rng: &mut Xoroshiro,
) {
    let Some(living) = target.get_living_entity() else {
        return;
    };
    let Some(name) = effect.get_string("to_apply") else {
        return;
    };
    let Some(selected) = StatusEffect::from_name(registry_path(name)) else {
        return;
    };
    let mut between = |min, max| -> Option<f32> {
        let min = number_provider(effect.get(min)?, level)?;
        let max = number_provider(effect.get(max)?, level)?;
        Some(min + rng.next_f32() * (max - min))
    };
    let (Some(seconds), Some(amplifier)) = (
        between("min_duration", "max_duration"),
        between("min_amplifier", "max_amplifier"),
    ) else {
        return;
    };
    let potion = Effect {
        effect_type: selected,
        duration: (seconds * 20.0).round() as i32,
        amplifier: amplifier.round().max(0.0) as u8,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: false,
    };
    if let Some(player) = target.get_player() {
        player.add_effect(potion);
    } else {
        living.add_effect(potion);
    }
}

// ExplodeEffect.apply reads the complete payload from the current registry entry.
fn explode(effect: &NbtCompound, level: i32, target: &dyn EntityBase) {
    let entity = target.get_entity();
    let world = entity.world.load_full();
    let interaction = match effect.get_string("block_interaction") {
        Some("trigger") => ExplosionInteraction::Trigger,
        Some("block") => ExplosionInteraction::Block,
        Some("mob") => ExplosionInteraction::Mob,
        Some("tnt") => ExplosionInteraction::Tnt,
        Some("none") => ExplosionInteraction::None,
        _ => return,
    };
    let Some(radius) = effect
        .get("radius")
        .and_then(|value| number_provider(value, level))
    else {
        return;
    };
    let damage_type = effect
        .get_string("damage_type")
        .and_then(|name| DamageType::from_name(registry_path(name)));
    if effect.get("damage_type").is_some() && damage_type.is_none() {
        return;
    }
    let immune_tag = effect.get_string("immune_blocks").and_then(|tag| {
        pumpkin_data::tag::get_latest_map(pumpkin_data::tag::RegistryKey::Block)
            .get(tag.trim_start_matches('#'))
            .copied()
    });
    let calculator = SimpleExplosionDamageCalculator::new(
        damage_type.is_some(),
        interaction != ExplosionInteraction::None,
        effect
            .get("knockback_multiplier")
            .and_then(|value| number_provider(value, level)),
        immune_tag,
    );
    let offset = effect
        .get_list("offset")
        .and_then(|list| {
            Some(Vector3::new(
                f64::from(list.first()?.as_numeric_float()?),
                f64::from(list.get(1)?.as_numeric_float()?),
                f64::from(list.get(2)?.as_numeric_float()?),
            ))
        })
        .unwrap_or_default();
    let position = entity.pos.load() + offset;
    let mut explosion = Explosion::new(
        radius.max(0.0),
        position,
        world.get_block_interaction(interaction),
    )
    .with_damage_calculator(Arc::new(calculator))
    .with_enchantment_settings(
        effect
            .get_bool("attribute_to_user")
            .unwrap_or(false)
            .then(|| world.get_entity_by_id(entity.entity_id))
            .flatten(),
        damage_type,
        effect.get_bool("create_fire").unwrap_or(false),
    );
    if let Some((small, large, sound)) = explosion_presentation(effect) {
        explosion = explosion.with_particles_and_sound(small, large, sound);
    }
    world.run_explosion(&explosion);
}

fn registry_path(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

// DamageEntity.apply decodes its damage identity from the same effect as its amount.
fn damage_entity_effect(
    effect: &NbtCompound,
    level: i32,
    rng: &mut Xoroshiro,
) -> Option<(DamageType, f32)> {
    let min = number_provider(effect.get("min_damage")?, level)?;
    let max = number_provider(effect.get("max_damage")?, level)?;
    let damage_type = DamageType::from_name(registry_path(effect.get_string("damage_type")?))?;
    Some((damage_type, min + rng.next_f32() * (max - min)))
}

fn explosion_presentation(effect: &NbtCompound) -> Option<(Particle, Particle, Sound)> {
    let particle =
        |key| Particle::from_name(registry_path(effect.get_compound(key)?.get_string("type")?));
    Some((
        particle("small_particle")?,
        particle("large_particle")?,
        Sound::from_name(registry_path(effect.get_string("sound")?))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchantment::definition::vanilla_enchantment_definitions;

    #[test]
    fn melee_registry_effects_decode_namespaced_types_particles_and_sound() {
        let definitions = vanilla_enchantment_definitions();
        let effect = |name| {
            definitions[name]
                .get_compound("effects")
                .unwrap()
                .get_list("minecraft:post_attack")
                .unwrap()[0]
                .extract_compound()
                .unwrap()
                .get_compound("effect")
                .unwrap()
        };
        let thorns = effect("thorns").get_list("effects").unwrap()[0]
            .extract_compound()
            .unwrap();
        let mut rng = Xoroshiro::from_seed(1);
        let (damage_type, damage) = damage_entity_effect(thorns, 3, &mut rng).unwrap();
        assert_eq!(damage_type, DamageType::THORNS);
        assert!((1.0..5.0).contains(&damage));
        assert_eq!(
            explosion_presentation(effect("wind_burst")),
            Some((
                Particle::GustEmitterSmall,
                Particle::GustEmitterLarge,
                Sound::EntityWindChargeWindBurst
            ))
        );
        assert_eq!(
            StatusEffect::from_name(registry_path(
                effect("bane_of_arthropods").get_string("to_apply").unwrap()
            )),
            Some(&StatusEffect::SLOWNESS)
        );
    }
}
