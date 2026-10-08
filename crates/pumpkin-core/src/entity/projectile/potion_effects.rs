use crate::entity::EntityBase;
use pumpkin_data::{
    damage::DamageType,
    effect::StatusEffect,
    entity::EntityType,
    potion::Effect,
    tag::{self, Taggable},
};
use pumpkin_util::math::boundingbox::BoundingBox;

// PotionContents.BASE_POTION_COLOR.
const BASE_POTION_COLOR: i32 = -13_083_194;

pub type EffectEntry = (&'static StatusEffect, i32, u8, bool, bool, bool);

pub fn affected_by_potions(entity: &dyn EntityBase) -> bool {
    entity.get_living_entity().is_some()
        && !entity.is_spectator()
        && entity.get_entity().entity_type != &EntityType::ARMOR_STAND
}

// AABB.distanceToSqr, used by ThrownSplashPotion.onHitAsPotion (box-to-box, not feet distance).
pub(super) fn box_distance_squared(a: BoundingBox, b: BoundingBox) -> f64 {
    let x = (a.min.x - b.max.x).max(b.min.x - a.max.x).max(0.0);
    let y = (a.min.y - b.max.y).max(b.min.y - a.max.y).max(0.0);
    let z = (a.min.z - b.max.z).max(b.min.z - a.max.z).max(0.0);
    x * x + y * y + z * z
}

// MobEffectInstance.mapDuration: infinite and zero durations must survive scaling unchanged.
pub fn scaled_duration(duration: i32, scale: f64) -> i32 {
    if duration == -1 || duration == 0 {
        duration
    } else {
        (f64::from(duration) * scale + 0.5) as i32
    }
}

// MobEffectInstance.withScaledDuration floors the component multiplier, with a one-tick minimum.
pub fn with_scaled_duration(duration: i32, scale: f32) -> i32 {
    if duration == -1 || duration == 0 {
        duration
    } else {
        ((duration as f32 * scale).floor() as i32).max(1)
    }
}

/// Applies a thrown potion/cloud effect with separate duration and instantaneous potency.
pub fn apply_effect(
    target: &dyn EntityBase,
    effect: EffectEntry,
    duration_scale: f64,
    instant_scale: f64,
    source: &dyn EntityBase,
    owner: Option<&dyn EntityBase>,
    splash: bool,
) {
    let Some(living) = target.get_living_entity() else {
        return;
    };
    let (effect_type, duration, amplifier, ambient, show_particles, show_icon) = effect;
    if effect_type == &StatusEffect::INSTANT_HEALTH || effect_type == &StatusEffect::INSTANT_DAMAGE
    {
        // HealOrHarmMobEffect.applyInstantaneousEffect.
        let inverted = target
            .get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_INVERTED_HEALING_AND_HARM);
        let harm = effect_type == &StatusEffect::INSTANT_DAMAGE;
        let healing = harm == inverted;
        let base: i32 = if healing { 4 } else { 6 };
        let amount =
            (instant_scale * f64::from(base.wrapping_shl(u32::from(amplifier))) + 0.5) as i32;
        if healing {
            // LivingEntity.heal accepts Java shift-overflow negatives, but never resurrects.
            if amount != 0
                && living.health.load() > 0.0
                && !living.dead.load(std::sync::atomic::Ordering::Relaxed)
            {
                living.heal(amount as f32);
            }
        } else {
            super::damage::hurt_entity(
                target,
                amount as f32,
                DamageType::INDIRECT_MAGIC,
                source,
                owner,
            );
        }
    } else {
        let duration = if splash {
            scaled_duration(duration, duration_scale)
        } else {
            with_scaled_duration(duration, duration_scale as f32)
        };
        if splash && duration != -1 && duration <= 20 {
            return;
        }
        living.add_effect(Effect {
            effect_type,
            duration,
            amplifier,
            ambient,
            show_particles,
            show_icon,
            blend: false,
        });
    }
}

// PotionContents.getColorOptional weights visible effects by amplifier + 1.
pub fn potion_color(stack: &pumpkin_data::item_stack::ItemStack, effects: &[EffectEntry]) -> i32 {
    if let Some(color) = stack
        .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        .and_then(|p| p.custom_color)
    {
        return color;
    }
    let (mut red, mut green, mut blue, mut total) = (0, 0, 0, 0);
    for (effect, _, amplifier, _, visible, _) in effects {
        if *visible {
            let weight = i32::from(*amplifier) + 1;
            red += ((effect.color >> 16) & 255) * weight;
            green += ((effect.color >> 8) & 255) * weight;
            blue += (effect.color & 255) * weight;
            total += weight;
        }
    }
    if total == 0 {
        BASE_POTION_COLOR
    } else {
        (0xFFu32 << 24) as i32 | ((red / total) << 16) | ((green / total) << 8) | (blue / total)
    }
}

pub(super) fn splash_event(
    entity: &crate::entity::Entity,
    stack: &pumpkin_data::item_stack::ItemStack,
) {
    use pumpkin_protocol::java::client::play::CWorldEvent;
    let effects = crate::item::potion::PotionContents::read_potion_effects(stack);
    // AbstractThrownPotion.onHit checks the base potion's instant effects for the event/sound.
    let instant = stack
        .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        .and_then(|contents| contents.potion_id)
        .and_then(|id| pumpkin_data::potion::Potion::from_id(id as u8))
        .is_some_and(|potion| {
            potion.effects.iter().any(|effect| {
                effect.effect_type == &StatusEffect::INSTANT_HEALTH
                    || effect.effect_type == &StatusEffect::INSTANT_DAMAGE
            })
        });
    let world = entity.world.load();
    let pos = entity.block_pos.load();
    world.broadcast_to_chunk(
        entity.chunk_pos.load(),
        &CWorldEvent::new(
            if instant { 2007 } else { 2002 },
            pos,
            potion_color(stack, &effects),
            false,
        ),
    );
    if entity.silent.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    world.broadcast_to_chunk(
        entity.chunk_pos.load(),
        &CWorldEvent::new(if instant { 1054 } else { 1053 }, pos, 0, false),
    );
}
