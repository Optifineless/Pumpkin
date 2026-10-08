//! `EnchantmentsByCostWithDifficulty.enchant` and `EnchantmentHelper.selectEnchantment`.

use pumpkin_data::{
    data_component_impl::EnchantableImpl,
    enchantment::Enchantment,
    enchantment_provider::{EnchantmentProvider, EnchantmentProviderKind},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_util::random::{RandomGenerator, RandomImpl, get_seed, xoroshiro128::Xoroshiro};

/// Applies a generated provider with vanilla's stack enchantability and random selection.
pub fn apply(provider: &EnchantmentProvider, stack: &mut ItemStack, special_multiplier: f32) {
    enchant(
        provider,
        stack,
        special_multiplier,
        &mut RandomGenerator::Xoroshiro(Xoroshiro::from_seed(get_seed())),
    );
}

fn enchant(
    provider: &EnchantmentProvider,
    stack: &mut ItemStack,
    multiplier: f32,
    random: &mut impl RandomImpl,
) {
    let selected = match provider.kind {
        EnchantmentProviderKind::Single { enchantment, level } => vec![(enchantment, level)],
        EnchantmentProviderKind::ByCost { enchantments, cost } => {
            select_enchantment(random, stack, cost, enchantments)
        }
        EnchantmentProviderKind::ByCostWithDifficulty {
            enchantments,
            min_cost,
            max_cost_span,
        } => {
            // EnchantmentsByCostWithDifficulty.enchant truncates the span, then samples inclusively.
            let cost = difficulty_cost(random, min_cost, max_cost_span, multiplier);
            select_enchantment(random, stack, cost, enchantments)
        }
    };
    for (enchantment, level) in selected {
        stack.enchant(enchantment, level);
    }
}

fn difficulty_cost(random: &mut impl RandomImpl, min: i32, span: i32, multiplier: f32) -> i32 {
    min + random.next_bounded_i32((multiplier * span as f32) as i32 + 1)
}

fn select_enchantment(
    random: &mut impl RandomImpl,
    stack: &ItemStack,
    mut cost: i32,
    source: &[&'static Enchantment],
) -> Vec<(&'static Enchantment, i32)> {
    let mut results = Vec::new();
    let Some(enchantable) = stack.get_data_component::<EnchantableImpl>() else {
        return results;
    };
    cost += 1
        + random.next_bounded_i32(enchantable.value / 4 + 1)
        + random.next_bounded_i32(enchantable.value / 4 + 1);
    let span = (random.next_f32() + random.next_f32() - 1.0) * 0.15;
    // Java Math.round(float) is floor(x + 0.5), and this expression is not fused.
    cost = ((cost as f32 + cost as f32 * span) + 0.5).floor().max(1.0) as i32;
    let mut available = available_enchantment_results(cost, stack, source);
    if let Some(first) = weighted_choice(random, &available) {
        results.push(first);
        while random.next_bounded_i32(50) <= cost {
            // EnchantmentHelper.filterCompatibleEnchantments filters against the last result.
            if let Some((last, _)) = results.last() {
                available.retain(|(enchantment, _)| last.are_compatible(enchantment));
            }
            let Some(next) = weighted_choice(random, &available) else {
                break;
            };
            results.push(next);
            cost /= 2;
        }
    }
    results
}

fn available_enchantment_results(
    cost: i32,
    stack: &ItemStack,
    source: &[&'static Enchantment],
) -> Vec<(&'static Enchantment, i32)> {
    // EnchantmentHelper.getAvailableEnchantmentResults uses primary items and both cost bounds.
    source
        .iter()
        .copied()
        .filter(|e| stack.item == &Item::BOOK || e.is_primary_item(stack.item))
        .filter_map(|enchantment| {
            (1..=enchantment.max_level)
                .rev()
                .find(|&level| {
                    cost >= enchantment.min_cost.calculate(level)
                        && cost <= enchantment.max_cost.calculate(level)
                })
                .map(|level| (enchantment, level))
        })
        .collect()
}

fn weighted_choice(
    random: &mut impl RandomImpl,
    available: &[(&'static Enchantment, i32)],
) -> Option<(&'static Enchantment, i32)> {
    // WeightedRandom.getRandomItem draws an integer in [0, totalWeight).
    let total: i32 = available.iter().map(|(e, _)| e.weight).sum();
    if total <= 0 {
        return None;
    }
    let mut roll = random.next_bounded_i32(total);
    for &(enchantment, level) in available {
        roll -= enchantment.weight;
        if roll < 0 {
            return Some((enchantment, level));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_util::random::legacy_rand::LegacyRand;

    #[test]
    fn review_difficulty_cost_samples_inclusive_truncated_range() {
        // Java Random seeds 0, 2, 5 give nextInt(9) = 6, 4, 8.
        // 0.5 * 17 truncates to 8, so the inclusive cost range is [5, 13].
        for (seed, expected) in [(0, 11), (2, 9), (5, 13)] {
            assert_eq!(
                difficulty_cost(&mut LegacyRand::from_seed(seed), 5, 17, 0.5),
                expected
            );
        }
    }

    fn names(selected: Vec<(&'static Enchantment, i32)>) -> Vec<(&'static str, i32)> {
        selected
            .into_iter()
            .map(|(enchantment, level)| (enchantment.registry_key, level))
            .collect()
    }

    #[test]
    fn review_seeded_selection_obeys_enchantability_costs_and_continuation() {
        let EnchantmentProviderKind::ByCostWithDifficulty { enchantments, .. } =
            EnchantmentProvider::MOB_SPAWN_EQUIPMENT.kind
        else {
            panic!("mob provider must be difficulty based")
        };
        // Java Random seed 42 and the stock provider's ordered tag: adjusted sword cost 12.
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        assert_eq!(
            names(select_enchantment(
                &mut LegacyRand::from_seed(42),
                &sword,
                10,
                enchantments
            )),
            vec![("fire_aspect", 1)]
        );
        // Java's same seed with bow cost 30 selects Power III, then Punch I.
        let bow = ItemStack::new(1, &Item::BOW);
        assert_eq!(
            names(select_enchantment(
                &mut LegacyRand::from_seed(42),
                &bow,
                30,
                enchantments
            )),
            vec![("power", 3), ("punch", 1)]
        );
        assert!(
            select_enchantment(&mut LegacyRand::from_seed(42), &bow, 1000, enchantments).is_empty()
        );
        assert!(
            select_enchantment(
                &mut LegacyRand::from_seed(42),
                &ItemStack::new(1, &Item::STONE),
                30,
                enchantments
            )
            .is_empty()
        );
    }

    #[test]
    fn review_provider_uses_random_cost_before_stack_selection() {
        let provider = EnchantmentProvider::MOB_SPAWN_EQUIPMENT;
        // Seed 2 draws cost 9; two bound-one rolls and .004156,.496823
        // yield adjusted cost 9. Only Power I is eligible; Punch and Flame are excluded.
        let mut bow = ItemStack::new(1, &Item::BOW);
        enchant(&provider, &mut bow, 0.5, &mut LegacyRand::from_seed(2));
        assert_eq!(bow.get_enchantment_level(&Enchantment::POWER), 1);
        assert_eq!(bow.get_enchantment_level(&Enchantment::PUNCH), 0);
        assert_eq!(bow.get_enchantment_level(&Enchantment::FLAME), 0);
    }
    #[test]
    fn generated_mob_provider_preserves_java_seed_zero_bow_selection() {
        let EnchantmentProviderKind::ByCostWithDifficulty {
            enchantments,
            min_cost,
            max_cost_span,
        } = EnchantmentProvider::MOB_SPAWN_EQUIPMENT.kind
        else {
            panic!("mob provider must be difficulty based")
        };
        let mut random = LegacyRand::from_seed(0);
        let cost = difficulty_cost(&mut random, min_cost, max_cost_span, 1.0);
        let selected = names(select_enchantment(
            &mut random,
            &ItemStack::new(1, &Item::BOW),
            cost,
            enchantments,
        ));
        // Java: adjusted cost 12, first weighted roll 16/17 selects Punch I.
        assert_eq!(selected.first(), Some(&("punch", 1)));
    }
}
