use super::*;
use pumpkin_data::{data_component::DataComponent, item::Item};

fn slot(count: i32, nutrition: Option<i32>) -> FoodSlot {
    FoodSlot { count, nutrition }
}

#[test]
fn villager_s2_owned_policy_matches_components_and_java_arithmetic() {
    for (item, expected) in [
        (&Item::BREAD, Some(4)),
        (&Item::CARROT, Some(1)),
        (&Item::POTATO, Some(1)),
        (&Item::BEETROOT, Some(1)),
        (&Item::WHEAT, None),
        (&Item::BAKED_POTATO, None),
        (&Item::STONE, None),
    ] {
        assert_eq!(stack_nutrition(&ItemStack::new(1, item)), expected);
    }
    let mut patched = ItemStack::new(3, &Item::STONE);
    patched.set_data_component(VillagerFoodImpl { nutrition: 7 });
    assert_eq!(count_food_points_in_inventory(&[patched.clone()]), 21);
    let mut removed = ItemStack::new(64, &Item::BREAD);
    removed.remove_data_component(DataComponent::VillagerFood);
    assert_eq!(stack_nutrition(&removed), None);
    assert_eq!(count_food_points_in_inventory(&[removed]), 0);
    patched.item_count = 0;
    assert_eq!(stack_nutrition(&patched), None);
    assert_eq!(count_food_points_in_inventory(&[patched]), 0);

    // Vanilla hasExcessFood/wantsMoreFood never consult cached FoodLevel.
    let cached_food = i32::MAX;
    for (count, wants, excess) in [
        (11, true, false),
        (12, false, false),
        (23, false, false),
        (24, false, true),
    ] {
        let stock = [ItemStack::new(count, &Item::CARROT)];
        assert_eq!(wants_more_food(&stock), wants);
        assert_eq!(has_excess_food(&stock), excess);
        assert_eq!(
            eat_until_full([slot(1, Some(4))], cached_food).1,
            cached_food
        );
    }

    let mut large = ItemStack::new(2, &Item::STONE);
    large.set_data_component(VillagerFoodImpl {
        nutrition: i32::MAX,
    });
    assert_eq!(count_food_points_in_inventory(&[large.clone()]), -2);
    assert_eq!(
        count_food_points_in_inventory(&[large, ItemStack::new(1, &Item::BREAD)]),
        2
    );
    assert_eq!(
        eat_and_digest_food([slot(2, Some(i32::MAX))], 10),
        ([slot(0, Some(i32::MAX))], -4)
    );
    assert_eq!(
        eat_until_full([slot(1, Some(i32::MAX))], 10),
        ([slot(0, Some(i32::MAX))], -2_147_483_639)
    );
}

#[test]
fn villager_s2_owned_eating_uses_slot_order_and_signed_values() {
    let slots = [slot(2, None), slot(3, Some(7)), slot(5, Some(4))];
    let consumed = [slot(2, None), slot(1, Some(7)), slot(5, Some(4))];
    assert_eq!(eat_until_full(slots, 0), (consumed, 14));
    assert_eq!(eat_and_digest_food(slots, 0), (consumed, 2));
    assert_eq!(eat_until_full(slots, 12), (slots, 12));
    assert_eq!(eat_and_digest_food(slots, 13), (slots, 1));
    assert_eq!(
        eat_and_digest_food([slot(1, Some(0)), slot(2, Some(-1))], 0),
        ([slot(0, Some(0)), slot(0, Some(-1))], -14)
    );
    assert_eq!(eat_and_digest_food([], i32::MIN), ([], 2_147_483_636));
}
