use super::LivingEntity;
use crate::entity::{
    NBTStorage, NBTStorageInit,
    attributes::{Modifier, ModifierOperation},
};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{
    attributes::Attributes, damage::DamageType, data_component_impl::Operation,
    effect::StatusEffect, entity::EntityType, potion::Effect,
};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::atomic::Ordering::Relaxed;

#[derive(Clone)]
pub(super) struct HiddenEffect {
    effect: Effect,
    hidden: Option<Box<Self>>,
}

// LivingEntity.canBeAffected preserves the override order from vanilla 26.3.
/// Checks the living entity type's effect immunity before consuming a cloud application.
pub fn can_be_affected(entity: &EntityType, effect: &StatusEffect) -> bool {
    // Spider/AbstractNautilus, WitherBoss/WitherSkeleton and Parched.canBeAffected.
    if (effect == &StatusEffect::POISON
        && [
            &EntityType::SPIDER,
            &EntityType::CAVE_SPIDER,
            &EntityType::NAUTILUS,
            &EntityType::ZOMBIE_NAUTILUS,
        ]
        .contains(&entity))
        || (effect == &StatusEffect::WITHER
            && [&EntityType::WITHER, &EntityType::WITHER_SKELETON].contains(&entity))
        || (effect == &StatusEffect::WEAKNESS && entity == &EntityType::PARCHED)
    {
        return false;
    }
    if entity.has_tag(&tag::EntityType::MINECRAFT_IMMUNE_TO_INFESTED) {
        effect != &StatusEffect::INFESTED
    } else if entity.has_tag(&tag::EntityType::MINECRAFT_IMMUNE_TO_OOZING) {
        effect != &StatusEffect::OOZING
    } else {
        !entity.has_tag(&tag::EntityType::MINECRAFT_IGNORES_POISON_AND_REGEN)
            || (effect != &StatusEffect::POISON && effect != &StatusEffect::REGENERATION)
    }
}

const fn shorter(a: &Effect, b: &Effect) -> bool {
    a.duration != -1 && (a.duration < b.duration || b.duration == -1)
}

// MobEffectInstance.update: hidden changes alone do not refresh the visible effect.
fn update_effect(
    current: &mut Effect,
    hidden: &mut Option<Box<HiddenEffect>>,
    next: &Effect,
) -> bool {
    let mut changed = false;
    if next.amplifier > current.amplifier {
        if shorter(next, current) {
            *hidden = Some(Box::new(HiddenEffect {
                effect: current.clone(),
                hidden: hidden.take(),
            }));
        }
        current.amplifier = next.amplifier;
        current.duration = next.duration;
        changed = true;
    } else if shorter(current, next) {
        if next.amplifier == current.amplifier {
            current.duration = next.duration;
            changed = true;
        } else if let Some(previous) = hidden {
            update_effect(&mut previous.effect, &mut previous.hidden, next);
        } else {
            *hidden = Some(Box::new(HiddenEffect {
                effect: next.clone(),
                hidden: None,
            }));
        }
    }
    if (!next.ambient && current.ambient) || changed {
        current.ambient = next.ambient;
        changed = true;
    }
    if next.show_particles != current.show_particles {
        current.show_particles = next.show_particles;
        changed = true;
    }
    if next.show_icon != current.show_icon {
        current.show_icon = next.show_icon;
        changed = true;
    }
    changed
}

fn tick_hidden(hidden: &mut Option<Box<HiddenEffect>>) {
    if let Some(hidden) = hidden {
        tick_hidden(&mut hidden.hidden);
        if hidden.effect.duration > 0 {
            hidden.effect.duration -= 1;
        }
    }
}

fn tick_effect_duration(effect: &mut Effect, hidden: &mut Option<Box<HiddenEffect>>) -> bool {
    tick_hidden(hidden);
    if effect.duration > 0 {
        effect.duration -= 1;
    }
    if effect.duration == 0
        && let Some(next) = hidden.take()
    {
        *effect = next.effect;
        *hidden = next.hidden;
        true
    } else {
        false
    }
}

// AbsorptionMobEffect.onEffectStarted (26.3, line 24), followed by setAbsorptionAmount's clamp.
fn absorption_on_effect_started(current: f32, amplifier: u8, maximum: f32) -> f32 {
    current.max(4.0 * (f32::from(amplifier) + 1.0)).min(maximum)
}

impl LivingEntity {
    pub(super) fn add_effect_impl(&self, effect: Effect) {
        // LivingEntity.addEffect / canBeAffected checks immunities before updating.
        // WitherBoss.addEffect and EnderDragon.addEffect reject the whole operation.
        if self.entity.entity_type == &EntityType::WITHER
            || self.entity.entity_type == &EntityType::ENDER_DRAGON
            || !can_be_affected(self.entity.entity_type, effect.effect_type)
        {
            return;
        }
        let mut effect_event =
            crate::plugin::api::events::entity::entity_potion_effect::EntityPotionEffectEvent::new(
                self.entity.entity_id,
                effect.effect_type.translation_key.to_string(),
                effect.duration,
                effect.amplifier,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut effect_event);
        }
        if effect_event.cancelled {
            return;
        }

        // Apply instant effects immediately before storing
        if effect.effect_type == &StatusEffect::INSTANT_HEALTH {
            // HealOrHarmMobEffect.applyEffectTick uses Java's masked int shift.
            let heal_amount = 4i32.wrapping_shl(u32::from(effect.amplifier)).max(0) as f32;
            if heal_amount > 0.0 {
                self.heal(heal_amount);
            }
            // Preserve Pumpkin's existing immediate instant-effect dispatch.
            return;
        } else if effect.effect_type == &StatusEffect::INSTANT_DAMAGE {
            let damage_amount = 6i32.wrapping_shl(u32::from(effect.amplifier)) as f32;
            let dyn_self = self
                .entity
                .world
                .load()
                .get_entity_by_id(self.entity.entity_id);
            if let Some(dyn_self) = dyn_self {
                let _ = dyn_self.damage(&*dyn_self, damage_amount, DamageType::MAGIC);
            }
            return;
        }

        // MobEffectInstance.update stores weaker, longer effects behind the visible one.
        let incoming_type = effect.effect_type;
        let incoming_amplifier = effect.amplifier;
        let updated = {
            let mut active = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut hidden = self
                .hidden_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match active.entry(effect.effect_type) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let current = entry.get_mut();
                    update_effect(
                        current,
                        hidden.entry(effect.effect_type).or_default(),
                        &effect,
                    )
                    .then(|| current.clone())
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    self.bump_effect_version(effect.effect_type);
                    Some(entry.insert(effect).clone())
                }
            }
        };
        if let Some(updated) = updated {
            self.on_effect_updated(&updated);
        }
        // LivingEntity.addEffect invokes the incoming effect's start hook even without an upgrade.
        if incoming_type == &StatusEffect::ABSORPTION {
            self.set_absorption(absorption_on_effect_started(
                self.absorption.load(),
                incoming_amplifier,
                self.get_attribute_value(&Attributes::MAX_ABSORPTION) as f32,
            ));
        }
    }

    fn on_effect_updated(&self, effect: &Effect) {
        // LivingEntity.onEffectAdded / onEffectUpdated reapplies modifiers only on a change.

        // Effects that modify attributes (ex. speed) should also update the
        // entity's attribute instances (server-side) and then notify clients.
        if !effect.effect_type.attribute_modifiers.is_empty() {
            // Apply each attribute modifier into the local AttributeInstance
            for m in effect.effect_type.attribute_modifiers {
                let id = m.id.to_string();
                let op = match m.operation {
                    Operation::AddValue => ModifierOperation::Add,
                    Operation::AddMultipliedBase => ModifierOperation::MultiplyBase,
                    Operation::AddMultipliedTotal => ModifierOperation::MultiplyTotal,
                };
                let scaled_amount = m.base_value * (f64::from(effect.amplifier) + 1.);
                let mod_inst = Modifier {
                    id,
                    amount: scaled_amount,
                    operation: op,
                    // Vanilla `MobEffect.addAttributeModifiers` adds these as permanent.
                    permanent: true,
                };

                self.update_attribute(m.attribute, |inst| {
                    inst.add_or_replace_modifier(mod_inst.clone());
                });
            }

            // Recompute packet modifiers from active effects for each affected attribute
            let mut touched_attrs: Vec<pumpkin_data::attributes::Attributes> = Vec::new();
            for m in effect.effect_type.attribute_modifiers {
                if !touched_attrs.iter().any(|a| a.id == m.attribute.id) {
                    touched_attrs.push(m.attribute.clone());
                }
            }

            if !touched_attrs.is_empty() {
                crate::entity::attributes::send_attribute_updates_for_living(self, touched_attrs);
            }
        }

        // LivingEntity.onAttributeUpdated clamps limits after an upgrade or hidden-effect restore.
        for modifier in effect.effect_type.attribute_modifiers {
            if modifier.attribute == &Attributes::MAX_HEALTH {
                let maximum = self.get_max_health();
                if self.health.load() > maximum {
                    self.set_health(maximum);
                }
            } else if modifier.attribute == &Attributes::MAX_ABSORPTION {
                let maximum = self.get_attribute_value(&Attributes::MAX_ABSORPTION) as f32;
                if self.absorption.load() > maximum {
                    self.set_absorption(maximum);
                }
            }
        }

        // Apply invisible effect
        if effect.effect_type == &StatusEffect::INVISIBILITY {
            self.entity.set_invisible(true);
        }

        // Apply glowing effect
        if effect.effect_type == &StatusEffect::GLOWING {
            self.entity.set_glowing(true);
        }

        // Broadcast effect to nearby players
        self.broadcast_effect(effect);
        self.sync_effect_particles();
    }

    // Called with active_effects held, so a remove/re-add cannot masquerade as the old instance.
    pub(super) fn bump_effect_version(&self, kind: &'static StatusEffect) {
        let mut versions = self
            .effect_versions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let version = versions.entry(kind).or_default();
        *version = version.wrapping_add(1);
    }

    pub(super) fn restore_loaded_effect_metadata(&self) {
        // LivingEntity.readAdditionalSaveData marks effects dirty, then
        // updateDataBeforeSync restores their visibility without replaying add hooks.
        self.entity
            .set_invisible(self.has_effect(&StatusEffect::INVISIBILITY));
        self.entity
            .set_glowing(self.has_effect(&StatusEffect::GLOWING));
        self.sync_effect_particles();
    }

    pub(super) fn tick_effects_impl(&self) {
        // MobEffectInstance.tickServer executes the periodic callback before duration/promotion.
        let snapshots: Vec<_> = {
            let effects = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let versions = self
                .effect_versions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            effects
                .values()
                .map(|effect| {
                    (
                        effect.clone(),
                        versions.get(&effect.effect_type).copied().unwrap_or(0),
                    )
                })
                .collect()
        };
        for (snapshot, version) in snapshots {
            let kind = snapshot.effect_type;
            if self
                .effect_versions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&kind)
                .copied()
                .unwrap_or(0)
                != version
            {
                continue;
            }
            let duration = if snapshot.duration == -1 {
                self.entity.age.load(Relaxed)
            } else {
                snapshot.duration
            };
            let remains = snapshot.duration == -1 || snapshot.duration > 0;
            let remains = remains
                && crate::entity::effect::get_mob_effect(kind).is_none_or(|effect| {
                    !effect.should_apply_effect_tick(duration, snapshot.amplifier)
                        || effect.apply_effect_tick(self, snapshot.amplifier)
                });
            let updated = {
                let mut effects = self
                    .active_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Damage can resurrect, clearing/replacing effects. Never publish a stale promotion.
                if self
                    .effect_versions
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&kind)
                    .copied()
                    .unwrap_or(0)
                    != version
                {
                    continue;
                }
                let Some(effect) = effects.get_mut(&kind) else {
                    continue;
                };
                remains.then(|| {
                    let mut hidden = self
                        .hidden_effects
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let promoted = tick_effect_duration(effect, hidden.entry(kind).or_default());
                    (effect.clone(), promoted)
                })
            };
            match updated {
                None => {
                    self.remove_effect(kind);
                }
                Some((effect, promoted)) => {
                    if effect.duration != -1 && effect.duration <= 0 {
                        self.remove_effect(kind);
                    } else if promoted {
                        self.on_effect_updated(&effect);
                    } else if effect.duration > 0 && effect.duration % 600 == 0 {
                        self.broadcast_effect(&effect);
                    }
                }
            }
        }
    }

    pub(super) fn write_hidden_effect(&self, effect: &Effect, nbt: &mut NbtCompound) {
        let hidden = self
            .hidden_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(Some(hidden)) = hidden.get(&effect.effect_type) {
            write_hidden(hidden, nbt);
        }
    }

    pub(super) fn read_hidden_effect(&self, effect: &Effect, nbt: &NbtCompound) {
        let hidden = read_hidden(effect.effect_type, nbt, 0);
        self.hidden_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(effect.effect_type, hidden);
    }
}

fn write_hidden(hidden: &HiddenEffect, parent: &mut NbtCompound) {
    let mut nbt = NbtCompound::new();
    hidden.effect.write_nbt(&mut nbt);
    nbt.child_tags.remove("id");
    if let Some(next) = &hidden.hidden {
        write_hidden(next, &mut nbt);
    }
    parent.put_compound("hidden_effect", nbt);
}

fn read_hidden(
    effect_type: &'static StatusEffect,
    parent: &NbtCompound,
    depth: usize,
) -> Option<Box<HiddenEffect>> {
    if depth >= pumpkin_data::data_component_impl::MAX_DEATH_STATUS_EFFECT_DEPTH {
        return None;
    }
    let mut nbt = parent.get_compound("hidden_effect")?.clone();
    nbt.put_string("id", effect_type.minecraft_name.to_owned());
    let effect = Effect::create_from_nbt(&mut nbt)?;
    let hidden = read_hidden(effect_type, &nbt, depth + 1);
    Some(Box::new(HiddenEffect { effect, hidden }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarting_absorption_preserves_existing_hearts_and_only_replenishes_its_level() {
        assert_eq!(absorption_on_effect_started(6.0, 0, 8.0), 6.0);
        assert_eq!(absorption_on_effect_started(1.0, 0, 8.0), 4.0);
        assert_eq!(absorption_on_effect_started(1.0, 1, 8.0), 8.0);
        assert_eq!(absorption_on_effect_started(6.0, 0, 4.0), 4.0);
    }

    fn regeneration(duration: i32, amplifier: u8) -> Effect {
        Effect {
            effect_type: &StatusEffect::REGENERATION,
            duration,
            amplifier,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        }
    }

    #[test]
    fn effect_upgrades_keep_the_longer_effect_hidden_and_tick_it() {
        let mut current = regeneration(100, 1);
        let mut hidden = None;
        assert!(!update_effect(
            &mut current,
            &mut hidden,
            &regeneration(20, 0)
        ));
        assert_eq!((current.duration, current.amplifier), (100, 1));
        assert!(update_effect(
            &mut current,
            &mut hidden,
            &regeneration(2, 2)
        ));
        assert!(!tick_effect_duration(&mut current, &mut hidden));
        assert!(tick_effect_duration(&mut current, &mut hidden));
        assert_eq!((current.duration, current.amplifier), (98, 1));
        assert!(!update_effect(
            &mut current,
            &mut hidden,
            &regeneration(200, 0)
        ));
        assert_eq!(hidden.as_ref().unwrap().effect.duration, 200);
        assert!(update_effect(
            &mut current,
            &mut hidden,
            &regeneration(-1, 1)
        ));
        assert_eq!(current.duration, -1);
        assert!(!update_effect(
            &mut current,
            &mut hidden,
            &regeneration(500, 1)
        ));
    }

    #[test]
    fn saved_effects_preserve_unsigned_amplifiers_in_visible_and_hidden_nbt() {
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", "minecraft:regeneration".into());
        nbt.put_byte("amplifier", -1);
        nbt.put_int("duration", 20);
        nbt.put_byte("show_icon", 1);
        let mut hidden = NbtCompound::new();
        hidden.put_byte("amplifier", -128);
        hidden.put_int("duration", 100);
        hidden.put_byte("show_icon", 1);
        nbt.put_compound("hidden_effect", hidden);
        let effect = Effect::create_from_nbt(&mut nbt).unwrap();
        assert_eq!(effect.amplifier, 255);
        let hidden = read_hidden(&StatusEffect::REGENERATION, &nbt, 0).unwrap();
        assert_eq!(hidden.effect.amplifier, 128);
        let mut output = NbtCompound::new();
        effect.write_nbt(&mut output);
        write_hidden(&hidden, &mut output);
        assert_eq!(output.get_byte("amplifier"), Some(-1));
        assert_eq!(
            output
                .get_compound("hidden_effect")
                .unwrap()
                .get_byte("amplifier"),
            Some(-128)
        );
    }

    #[test]
    fn effect_immunities_match_base_tags_and_entity_overrides() {
        assert!(!can_be_affected(
            &EntityType::ZOMBIE,
            &StatusEffect::REGENERATION
        ));
        assert!(!can_be_affected(&EntityType::SPIDER, &StatusEffect::POISON));
        assert!(!can_be_affected(
            &EntityType::WITHER_SKELETON,
            &StatusEffect::WITHER
        ));
        assert!(!can_be_affected(
            &EntityType::PARCHED,
            &StatusEffect::WEAKNESS
        ));
        assert!(!can_be_affected(
            &EntityType::SILVERFISH,
            &StatusEffect::INFESTED
        ));
        assert!(!can_be_affected(&EntityType::SLIME, &StatusEffect::OOZING));
        assert!(can_be_affected(
            &EntityType::PLAYER,
            &StatusEffect::REGENERATION
        ));
    }
}
