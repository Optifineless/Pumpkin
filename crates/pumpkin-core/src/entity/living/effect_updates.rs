use super::LivingEntity;
use crate::entity::{
    Flag,
    attributes::{Modifier, ModifierOperation, send_attribute_updates_for_living},
};
use pumpkin_data::{
    attributes::Attributes, data_component_impl::Operation, effect::StatusEffect,
    entity::EntityType, potion::Effect, tracked_data,
};
use std::sync::atomic::Ordering::Relaxed;

impl LivingEntity {
    pub(super) fn set_absorption_state(&self, amount: f32) {
        let amount = amount.max(0.0);
        self.damage_owner.changed(0.0);
        self.absorption.store(amount);
        if self.entity.entity_type == &EntityType::PLAYER {
            self.entity
                .set_synced_data(tracked_data::player::DATA_PLAYER_ABSORPTION_ID, amount);
        }
    }

    // Caller owns damage through the active-effect change and this synchronous hook.
    // LivingEntity.onEffectAdded/onEffectUpdated and MobEffect.addAttributeModifiers.
    pub(super) fn commit_effect_update(&self, effect: &Effect) -> Vec<Attributes> {
        let mut touched_attrs = Vec::new();
        for modifier in effect.effect_type.attribute_modifiers {
            let operation = match modifier.operation {
                Operation::AddValue => ModifierOperation::Add,
                Operation::AddMultipliedBase => ModifierOperation::MultiplyBase,
                Operation::AddMultipliedTotal => ModifierOperation::MultiplyTotal,
            };
            let modifier_instance = Modifier {
                id: modifier.id.to_string(),
                amount: modifier.base_value * (f64::from(effect.amplifier) + 1.),
                operation,
                // Vanilla MobEffect.addAttributeModifiers adds these as permanent.
                permanent: true,
            };
            self.update_attribute(modifier.attribute, |instance| {
                instance.add_or_replace_modifier(modifier_instance);
            });
            if !touched_attrs.contains(modifier.attribute) {
                touched_attrs.push(modifier.attribute.clone());
            }
        }

        // LivingEntity.onAttributeUpdated clamps after upgrades and hidden-effect restores.
        for modifier in effect.effect_type.attribute_modifiers {
            if modifier.attribute == &Attributes::MAX_HEALTH {
                let maximum = self.get_max_health();
                if self.health.load() > maximum {
                    self.set_health(maximum);
                }
            } else if modifier.attribute == &Attributes::MAX_ABSORPTION {
                let maximum = self.get_attribute_value(&Attributes::MAX_ABSORPTION) as f32;
                if self.absorption.load() > maximum {
                    self.set_absorption_state(maximum);
                }
            }
        }
        if effect.effect_type == &StatusEffect::INVISIBILITY {
            self.entity.invisible.store(true, Relaxed);
            self.entity.set_flag_state(Flag::Invisible, true);
        }
        if effect.effect_type == &StatusEffect::GLOWING {
            self.entity.glowing.store(true, Relaxed);
            self.entity.set_flag_state(Flag::Glowing, true);
        }
        self.sync_effect_particles();
        touched_attrs
    }

    pub(super) fn publish_effect_update(
        &self,
        effect: Option<&Effect>,
        touched_attrs: Vec<Attributes>,
    ) {
        if !touched_attrs.is_empty() {
            send_attribute_updates_for_living(self, touched_attrs);
        }
        if let Some(effect) = effect {
            if effect.effect_type == &StatusEffect::INVISIBILITY
                || effect.effect_type == &StatusEffect::GLOWING
            {
                self.entity.send_bedrock_flags();
            }
            // Packet callbacks may have cleared or replaced the committed effect.
            if let Some(current) = self.get_effect(effect.effect_type) {
                self.broadcast_effect(&current);
            }
        }
    }

    pub(super) fn on_effect_updated(&self, effect: &Effect) {
        let owner = self.own_damage();
        let touched_attrs = self.commit_effect_update(effect);
        drop(owner);
        self.publish_effect_update(Some(effect), touched_attrs);
    }
}
