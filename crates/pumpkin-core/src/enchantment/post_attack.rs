use super::{
    conditions::matches_requirements,
    definition::{definition_matches_slot, enchantment_definition},
    helper::EnchantmentHelper,
};
use crate::entity::{EntityBase, equipment_damage::EquippedItem};
use pumpkin_data::{
    damage::DamageType, data_component_impl::EquipmentSlot, enchantment::EnchantmentTarget,
};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::random::{get_seed, xoroshiro128::Xoroshiro};

/// Identifies the owner and direct source of a landed attack for enchantment effects.
#[derive(Clone, Copy)]
pub struct AttackEffectContext<'a> {
    pub attacker: Option<&'a dyn EntityBase>,
    pub damaging_entity: Option<&'a dyn EntityBase>,
    pub damage_type: DamageType,
}

impl<'a> AttackEffectContext<'a> {
    /// Constructs a direct melee context; use separate sources for projectiles.
    pub fn melee(attacker: &'a dyn EntityBase, damage_type: DamageType) -> Self {
        Self {
            attacker: Some(attacker),
            damaging_entity: Some(attacker),
            damage_type,
        }
    }

    fn target(
        self,
        target: EnchantmentTarget,
        victim: &'a dyn EntityBase,
    ) -> Option<&'a dyn EntityBase> {
        select_target(target, victim, self.attacker, self.damaging_entity)
    }
}

impl EnchantmentHelper {
    /// Runs victim equipment effects, then attacking-item effects, on a landed hit.
    /// Mirrors EnchantmentHelper.doPostAttackEffectsWithItemSourceOnBreak. Sources
    /// without a living owner are skipped; their item-break callback is not available.
    pub fn on_post_attack(
        victim: &dyn EntityBase,
        context: AttackEffectContext<'_>,
        weapon: Option<&EquippedItem>,
    ) {
        if victim.get_living_entity().is_some() {
            // No inventory guard spans effect execution.
            for slot in [
                EquipmentSlot::MAIN_HAND,
                EquipmentSlot::OFF_HAND,
                EquipmentSlot::FEET,
                EquipmentSlot::LEGS,
                EquipmentSlot::CHEST,
                EquipmentSlot::HEAD,
                EquipmentSlot::BODY,
                EquipmentSlot::SADDLE,
            ] {
                let item = EquippedItem::capture(victim, &slot);
                Self::post_attack_on_item(
                    victim,
                    context,
                    &item,
                    victim,
                    EnchantmentTarget::Victim,
                );
            }
        }
        if let Some(weapon) = weapon
            && let Some(owner) = context.attacker
            && owner.get_living_entity().is_some()
        {
            Self::post_attack_on_item(victim, context, weapon, owner, EnchantmentTarget::Attacker);
        }
    }

    fn post_attack_on_item(
        victim: &dyn EntityBase,
        context: AttackEffectContext<'_>,
        item: &EquippedItem,
        owner: &dyn EntityBase,
        enchanted: EnchantmentTarget,
    ) {
        if item.stack.is_empty() {
            return;
        }
        let world = victim.get_entity().world.load_full();
        let mut rng = Xoroshiro::from_seed(get_seed());
        Self::run_iteration_on_item(&item.stack, |enchantment, level| {
            let Some(definition) = enchantment_definition(Some(&world), enchantment) else {
                return;
            };
            for_each_post_attack_effect(&definition, &item.slot, enchanted, |affected, effect| {
                if effect.get("requirements").is_some_and(|requirements| {
                    requirements.extract_compound().is_none_or(|requirements| {
                        matches_requirements(requirements, level, victim, context, &mut rng)
                            != Some(true)
                    })
                }) {
                    return;
                }
                if let Some(target) = context.target(affected, victim)
                    && let Some(payload) = effect.get_compound("effect")
                {
                    super::post_attack_effects::apply(
                        payload, level, owner, item, target, &mut rng,
                    );
                }
            });
        });
    }
}

// Enchantment.doPostAttack reads targets, requirements and effects from one entry.
fn for_each_post_attack_effect(
    definition: &NbtCompound,
    slot: &EquipmentSlot,
    enchanted: EnchantmentTarget,
    mut visitor: impl FnMut(EnchantmentTarget, &NbtCompound),
) {
    if !definition_matches_slot(definition, slot) {
        return;
    }
    for effect in definition
        .get_compound("effects")
        .and_then(|effects| effects.get_list("minecraft:post_attack"))
        .into_iter()
        .flatten()
    {
        let Some(effect) = effect.extract_compound() else {
            continue;
        };
        if parse_target(effect.get_string("enchanted")) == Some(enchanted)
            && let Some(affected) = parse_target(effect.get_string("affected"))
        {
            visitor(affected, effect);
        }
    }
}

fn parse_target(name: Option<&str>) -> Option<EnchantmentTarget> {
    match name? {
        "attacker" => Some(EnchantmentTarget::Attacker),
        "damaging_entity" => Some(EnchantmentTarget::DamagingEntity),
        "victim" => Some(EnchantmentTarget::Victim),
        _ => None,
    }
}

// EnchantmentTarget selects the affected entity independently of item ownership.
const fn select_target<'a, T: ?Sized>(
    target: EnchantmentTarget,
    victim: &'a T,
    attacker: Option<&'a T>,
    direct_source: Option<&'a T>,
) -> Option<&'a T> {
    match target {
        EnchantmentTarget::Attacker => attacker,
        EnchantmentTarget::DamagingEntity => direct_source,
        EnchantmentTarget::Victim => Some(victim),
    }
}

#[cfg(test)]
mod tests {
    use super::super::definition::vanilla_enchantment_definitions;
    use super::*;
    use pumpkin_nbt::tag::NbtTag;

    #[test]
    fn melee_registry_reordering_preserves_payload_and_direct_source_targeting() {
        let victim = 10;
        let attacker = 20;
        let direct = 30;
        let mut definition = vanilla_enchantment_definitions()["fire_aspect"].clone();
        let mut effects = definition.get_compound("effects").unwrap().clone();
        let original = effects.get_list("minecraft:post_attack").unwrap()[0].clone();
        let mut added = original.extract_compound().unwrap().clone();
        added.put_string("affected", "damaging_entity".into());
        added.put_string("enchanted", "attacker".into());
        let mut payload = added.get_compound("effect").unwrap().clone();
        payload.put_float("duration", 17.0);
        added.put("effect", NbtTag::Compound(payload));
        effects.put_list(
            "minecraft:post_attack",
            vec![NbtTag::Compound(added), original],
        );
        definition.put("effects", NbtTag::Compound(effects));
        let mut selected = Vec::new();
        for_each_post_attack_effect(
            &definition,
            &EquipmentSlot::MAIN_HAND,
            EnchantmentTarget::Attacker,
            |target, effect| {
                selected.push((
                    *select_target(target, &victim, Some(&attacker), Some(&direct)).unwrap(),
                    effect.get_compound("effect").unwrap().clone(),
                ));
            },
        );
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].0, direct);
        assert_eq!(selected[0].1.get_float("duration"), Some(17.0));
        assert_eq!(selected[1].0, victim);
        assert_eq!(
            select_target(
                EnchantmentTarget::Attacker,
                &victim,
                Some(&attacker),
                Some(&direct)
            ),
            Some(&attacker)
        );
        assert_eq!(
            select_target(
                EnchantmentTarget::DamagingEntity,
                &victim,
                Some(&attacker),
                None
            ),
            None
        );
    }
}
