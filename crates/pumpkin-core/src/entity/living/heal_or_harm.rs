use super::LivingEntity;
use crate::entity::EntityBase;
use pumpkin_data::{
    damage::DamageType,
    effect::StatusEffect,
    tag::{self, Taggable},
};

impl LivingEntity {
    /// Applies instant healing/harming with undead inversion under combat ownership.
    /// `None` selects the effect-tick rule; `Some(scale)` selects potion potency and rounding.
    pub(crate) fn apply_heal_or_harm(
        &self,
        effect: &StatusEffect,
        amplifier: u8,
        scale: Option<f64>,
    ) -> bool {
        if effect != &StatusEffect::INSTANT_HEALTH && effect != &StatusEffect::INSTANT_DAMAGE {
            return false;
        }
        let _owner = self.own_damage();
        // HealOrHarmMobEffect.applyEffectTick/applyInstantaneousEffect and
        // LivingEntity.isInvertedHealAndHarm (1055): the entity-type tag selects inversion.
        let heals = (effect == &StatusEffect::INSTANT_DAMAGE)
            == self
                .entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_INVERTED_HEALING_AND_HARM);
        let potency = (if heals { 4i32 } else { 6i32 }).wrapping_shl(u32::from(amplifier));
        let amount = scale.map_or_else(
            || if heals { potency.max(0) } else { potency },
            |scale| (scale * f64::from(potency) + 0.5) as i32,
        ) as f32;
        if heals {
            self.heal(amount);
        } else {
            let entity = self
                .entity
                .world
                .load()
                .get_entity_by_id(self.entity.entity_id);
            let caller: &dyn EntityBase = entity.as_deref().unwrap_or(self);
            caller.damage(caller, amount, DamageType::MAGIC);
        }
        true
    }
}
