use super::{
    LootContextParameters, LootRandom, LootTable, LootTableHandle, generate_with_random,
    with_random,
};
use pumpkin_data::item_stack::ItemStack;
fn place_items_in_chest(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    mut items_to_place: Vec<ItemStack>,
    rng: &mut LootRandom<'_>,
) {
    let inv_size = inventory.size();

    let mut available_slots: Vec<usize> = (0..inv_size)
        .filter(|&slot| inventory.get_stack(slot).is_empty())
        .collect();

    for i in (1..available_slots.len()).rev() {
        let j = rng.next_bounded_i32((i + 1) as i32) as usize;
        available_slots.swap(i, j);
    }

    shuffle_and_split_items(&mut items_to_place, available_slots.len(), rng);

    for item in items_to_place {
        let Some(slot) = available_slots.pop() else {
            tracing::warn!("Tried to over-fill a container");
            return;
        };
        inventory.set_stack(slot, item);
    }
}

pub fn fill_chest_inventory_handle(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    handle: &LootTableHandle,
    seed: i64,
) {
    fill_chest_inventory_with_context(inventory, handle, seed, &LootContextParameters::default());
}

/// Fill a container using one random source for loot rolls, splitting and placement.
pub fn fill_chest_inventory_with_context(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    handle: &LootTableHandle,
    seed: i64,
    params: &LootContextParameters,
) {
    // LootTable.fill continues the LootContext random source through slot shuffling.
    with_random(handle.parsed(), seed, params, &mut |rng| {
        let items = generate_with_random(handle.parsed(), params, rng);
        place_items_in_chest(inventory, items, rng);
    });
}

pub fn fill_chest_inventory(
    inventory: &std::sync::Arc<dyn pumpkin_inventory::Inventory>,
    table: &LootTable,
    seed: i64,
) {
    let params = LootContextParameters::default();
    with_random(table.parsed(), seed, &params, &mut |rng| {
        let items = generate_with_random(table.parsed(), &params, rng);
        place_items_in_chest(inventory, items, rng);
    });
}

fn shuffle_and_split_items(
    result: &mut Vec<ItemStack>,
    available_slots: usize,
    rng: &mut LootRandom<'_>,
) {
    let mut splittable: Vec<ItemStack> = Vec::new();
    let mut i = 0;
    while i < result.len() {
        if result[i].is_empty() {
            result.remove(i);
        } else if result[i].item_count > 1 {
            splittable.push(result.remove(i));
        } else {
            i += 1;
        }
    }

    while available_slots > result.len() + splittable.len() && !splittable.is_empty() {
        let idx = if splittable.len() == 1 {
            0
        } else {
            rng.next_bounded_i32(splittable.len() as i32) as usize
        };
        let mut stack = splittable.remove(idx);

        let count = stack.item_count as i32;
        // LootTable.shuffleAndSplitItems uses Mth.nextInt, which does not draw for equal bounds.
        let split_off = if count / 2 == 1 {
            1
        } else {
            1 + rng.next_bounded_i32(count / 2)
        };
        stack.item_count = (count - split_off) as u8;
        let mut copy = stack.clone();
        copy.item_count = split_off as u8;

        if stack.item_count > 1 && rng.next_bool() {
            splittable.push(stack);
        } else {
            result.push(stack);
        }
        if copy.item_count > 1 && rng.next_bool() {
            splittable.push(copy);
        } else {
            result.push(copy);
        }
    }

    result.extend(splittable);

    let n = result.len();
    for i in (1..n).rev() {
        let j = rng.next_bounded_i32((i + 1) as i32) as usize;
        result.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::{LootRandom, place_items_in_chest};
    #[test]
    fn empty_container_loot_still_advances_the_slot_shuffle() {
        // Actual 26.3 LootTable.getAvailableSlots for nine empty slots, RandomSource.create(37).
        let inventory: std::sync::Arc<dyn pumpkin_inventory::Inventory> =
            std::sync::Arc::new(pumpkin_inventory::SimpleInventory::new(9));
        let mut rng = LootRandom::seeded(37);
        place_items_in_chest(&inventory, Vec::new(), &mut rng);
        assert_eq!(rng.next_bounded_i32(1000), 259);
    }
}
