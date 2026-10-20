//! Component food policy over caller-owned values, pending the slice 2b storage owner.

use pumpkin_data::{data_component_impl::VillagerFoodImpl, item_stack::ItemStack};

use super::data::BREEDING_FOOD_THRESHOLD;

const EXCESS_FOOD_THRESHOLD: i32 = 24;

/// Returns the effective component nutrition, including patches and removals.
#[must_use]
pub fn stack_nutrition(stack: &ItemStack) -> Option<i32> {
    if stack.is_empty() {
        return None;
    }
    stack
        .get_data_component::<VillagerFoodImpl>()
        .map(|food| food.nutrition)
}

// Villager.countFoodPointsInInventory uses Java signed-int arithmetic.
#[must_use]
pub fn count_food_points_in_inventory(stacks: &[ItemStack]) -> i32 {
    stacks.iter().fold(0i32, |points, stack| {
        points.wrapping_add(
            i32::from(stack.item_count).wrapping_mul(stack_nutrition(stack).unwrap_or(0)),
        )
    })
}

#[must_use]
pub fn has_excess_food(stacks: &[ItemStack]) -> bool {
    count_food_points_in_inventory(stacks) >= EXCESS_FOOD_THRESHOLD
}

#[must_use]
pub fn wants_more_food(stacks: &[ItemStack]) -> bool {
    count_food_points_in_inventory(stacks) < BREEDING_FOOD_THRESHOLD
}

/// One pre-resolved slot owned by the caller, with no live inventory authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoodSlot {
    pub count: i32,
    pub nutrition: Option<i32>,
}

/// Mirrors Villager.eatUntilFull over owned scalars, preserving signed stream values.
#[must_use]
pub fn eat_until_full<const N: usize>(
    mut slots: [FoodSlot; N],
    mut food_level: i32,
) -> ([FoodSlot; N], i32) {
    if food_level >= BREEDING_FOOD_THRESHOLD {
        return (slots, food_level);
    }
    for slot in &mut slots {
        let Some(nutrition) = slot.nutrition else {
            continue;
        };
        for _ in 0..slot.count {
            food_level = food_level.wrapping_add(nutrition);
            slot.count = slot.count.wrapping_sub(1);
            if food_level >= BREEDING_FOOD_THRESHOLD {
                return (slots, food_level);
            }
        }
    }
    (slots, food_level)
}

/// Mirrors Villager.eatAndDigestFood; callers must supply a future admitted snapshot.
#[must_use]
pub fn eat_and_digest_food<const N: usize>(
    slots: [FoodSlot; N],
    food_level: i32,
) -> ([FoodSlot; N], i32) {
    let (slots, food_level) = eat_until_full(slots, food_level);
    (slots, food_level.wrapping_sub(BREEDING_FOOD_THRESHOLD))
}

#[cfg(test)]
mod tests;
