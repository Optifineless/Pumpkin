use super::LivingEntity;
use crate::{
    enchantment::{
        EnchantmentHelper,
        conditions::matches_requirements,
        definition::{definition_matches_slot, enchantment_definition},
        post_attack::AttackEffectContext,
    },
    entity::{
        EntityBase, death_loot::equipment_slots_in_vanilla_order, equipment_damage::EquippedItem,
    },
};
use pumpkin_data::damage::DamageType;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::random::{get_seed, xoroshiro128::Xoroshiro};

#[cfg(test)]
thread_local! {
    pub(super) static TEST_RANDOM: std::cell::RefCell<Option<Xoroshiro>> = const { std::cell::RefCell::new(None) };
}

impl LivingEntity {
    // LivingEntity.isInvulnerableTo / EnchantmentHelper.isImmuneToDamage / Enchantment.isImmuneToDamage.
    pub(super) fn is_immune_to_enchantment_damage(
        &self,
        victim: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        #[cfg(test)]
        if let Some(result) = TEST_RANDOM.with(|random| {
            random
                .borrow_mut()
                .as_mut()
                .map(|rng| self.immune_with_random(victim, damage_type, source, cause, rng))
        }) {
            return result;
        }
        self.immune_with_random(
            victim,
            damage_type,
            source,
            cause,
            &mut Xoroshiro::from_seed(get_seed()),
        )
    }

    fn immune_with_random(
        &self,
        victim: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
        rng: &mut Xoroshiro,
    ) -> bool {
        let world = self.entity.world.load();
        for slot in equipment_slots_in_vanilla_order() {
            let item = EquippedItem::capture(victim, &slot);
            let mut immune = false;
            EnchantmentHelper::run_iteration_on_item(&item.stack, |enchantment, level| {
                if let Some(definition) = enchantment_definition(Some(&world), enchantment)
                    && definition_matches_slot(&definition, &slot)
                    && let Some(effects) = definition.get_compound("effects")
                    && let Some(immunities) = effects.get_list("minecraft:damage_immunity")
                {
                    // EnchantmentHelper.isImmuneToDamage:160 skips later enchantments once immune.
                    immune = immune
                        || immunities.iter().any(|effect| {
                            effect.extract_compound().is_some_and(|effect| {
                                effect
                                    .get_compound("requirements")
                                    .is_none_or(|requirements| {
                                        let mut requirements = requirements.clone();
                                        normalize_tag_keys(&mut requirements);
                                        matches_requirements(
                                            &requirements,
                                            level,
                                            victim,
                                            AttackEffectContext {
                                                attacker: cause,
                                                damaging_entity: source,
                                                damage_type,
                                            },
                                            rng,
                                        ) == Some(true)
                                    })
                            })
                        });
                }
            });
            if immune {
                return true;
            }
        }
        false
    }
}

// TagKey.CODEC accepts the '#' prefix; the shared NBT predicate evaluator expects a tag name.
fn normalize_tag_keys(requirements: &mut NbtCompound) {
    for (key, value) in &mut requirements.child_tags {
        match value {
            NbtTag::String(name) if key.as_ref() == "id" && name.starts_with('#') => {
                *name = name.trim_start_matches('#').into();
            }
            NbtTag::Compound(compound) => normalize_tag_keys(compound),
            NbtTag::List(values) => {
                for value in values {
                    if let NbtTag::Compound(compound) = value {
                        normalize_tag_keys(compound);
                    }
                }
            }
            _ => {}
        }
    }
}
