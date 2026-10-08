use rand::{Rng, RngExt};

/// Rounds one recipe's XP using `AbstractFurnaceBlockEntity.createExperience`'s random fraction.
pub fn recipe_experience(amount: u32, value: f32, random: &mut impl Rng) -> i32 {
    let total = amount as f32 * value;
    let floor = total.floor();
    let fraction = total - floor;
    floor as i32 + i32::from(fraction != 0.0 && random.random::<f32>() < fraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn fractional_recipe_xp_can_round_up_or_down() {
        let mut results = std::collections::BTreeSet::new();
        for seed in 0..100 {
            results.insert(recipe_experience(3, 0.5, &mut StdRng::seed_from_u64(seed)));
        }
        assert_eq!(results, [1, 2].into_iter().collect());
    }
}
