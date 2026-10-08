use super::{
    EnchantmentHelper,
    conditions::{matches_requirements, number_provider},
    definition::enchantment_definition,
    post_attack::AttackEffectContext,
};
use crate::entity::EntityBase;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::random::{RandomImpl, get_seed, xoroshiro128::Xoroshiro};

impl EnchantmentHelper {
    /// Evaluates a weapon's damage or knockback effects with the actual hit context.
    // EnchantmentHelper.modifyDamage / modifyKnockback -> Enchantment.modifyDamageFilteredValue.
    pub fn modify_projectile_value(
        weapon: &ItemStack,
        target: &dyn EntityBase,
        context: AttackEffectContext<'_>,
        effect_key: &str,
        mut value: f32,
    ) -> f32 {
        let world = target.get_entity().world.load();
        let mut rng = Xoroshiro::from_seed(get_seed());
        Self::run_iteration_on_item(weapon, |enchantment, level| {
            let Some(definition) = enchantment_definition(Some(&world), enchantment) else {
                return;
            };
            for entry in definition
                .get_compound("effects")
                .and_then(|effects| effects.get_list(effect_key))
                .into_iter()
                .flatten()
            {
                let Some(entry) = entry.extract_compound() else {
                    continue;
                };
                if entry.get("requirements").is_some_and(|requirements| {
                    requirements.extract_compound().is_none_or(|requirements| {
                        matches_requirements(requirements, level, target, context, &mut rng)
                            != Some(true)
                    })
                }) {
                    continue;
                }
                if let Some(effect) = entry.get_compound("effect")
                    && let Some(result) = process_value(effect, level, value, &mut rng)
                {
                    value = result;
                }
            }
        });
        value
    }
}

// EnchantmentValueEffect implementations use float values, including sequential AllOf.ValueEffects.
fn process_value(effect: &NbtCompound, level: i32, value: f32, rng: &mut Xoroshiro) -> Option<f32> {
    match effect.get_string("type")? {
        "minecraft:add" => Some(value + number_provider(effect.get("value")?, level)?),
        "minecraft:multiply" => Some(value * number_provider(effect.get("factor")?, level)?),
        "minecraft:set" => number_provider(effect.get("value")?, level),
        "minecraft:all_of" => {
            let mut result = value;
            for child in effect.get_list("effects")? {
                result = process_value(child.extract_compound()?, level, result, rng)?;
            }
            Some(result)
        }
        "minecraft:remove_binomial" => {
            let chance = number_provider(effect.get("chance")?, level)?;
            // RemoveBinomial.process uses a normal approximation for large, balanced inputs.
            let removed =
                if value > 128.0 && value * chance >= 20.0 && value * (1.0 - chance) >= 20.0 {
                    let mean = f64::from((value * chance).floor());
                    let deviation = f64::from(value * chance * (1.0 - chance)).sqrt();
                    ((mean + rng.next_gaussian() * deviation + 0.5).floor() as i32)
                        .clamp(0, value as i32)
                } else {
                    (0..value.ceil() as i32)
                        .filter(|_| rng.next_f32() < chance)
                        .count() as i32
                };
            Some(value - removed as f32)
        }
        _ => None,
    }
}
