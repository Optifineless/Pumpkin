use pumpkin_data::{
    item::Item,
    item_stack::ItemStack,
    recipes::{CraftingRecipeTypes, RecipeIngredientTypes, RecipeResultStruct},
    tag::{self, Taggable},
};
use pumpkin_protocol::codec::recipe::OwnedCraftingRecipe;

use super::{
    crafting_screen_handler::RecipeResult,
    recipe_provider::{GenericRecipe, IngredientRef},
    recipes::RecipeInputInventory,
    special_recipes,
};

pub(super) fn recipe_matches(
    recipe: GenericRecipe<'_>,
    height: usize,
    width: usize,
    left: usize,
    top: usize,
    count: usize,
    inventory: &dyn RecipeInputInventory,
) -> Option<RecipeResult> {
    let shape = Shape {
        width,
        height,
        left,
        top,
    };
    let stack = match recipe {
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingShaped {
            key,
            pattern,
            result,
            ..
        }) => {
            let keys: Vec<_> = key
                .iter()
                .map(|(c, i)| (*c, IngredientRef::Vanilla(i)))
                .collect();
            shaped(pattern, &keys, &shape, count, inventory).then(|| result.assemble(None, 0))?
        }
        GenericRecipe::Dynamic(OwnedCraftingRecipe::Shaped {
            key,
            pattern,
            result,
            ..
        }) => {
            let keys: Vec<_> = key
                .iter()
                .map(|(c, i)| (*c, IngredientRef::Dynamic(i)))
                .collect();
            let pattern: Vec<_> = pattern.iter().map(String::as_str).collect();
            shaped(&pattern, &keys, &shape, count, inventory)
                .then(|| plain_stack(&result.item_id, result.count))?
        }
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingShapeless {
            ingredients,
            result,
            ..
        }) => {
            let ingredients: Vec<_> = ingredients.iter().map(IngredientRef::Vanilla).collect();
            shapeless(&ingredients, count, inventory).then(|| result.assemble(None, 0))?
        }
        GenericRecipe::Dynamic(OwnedCraftingRecipe::Shapeless {
            ingredients,
            result,
            ..
        }) => {
            let ingredients: Vec<_> = ingredients.iter().map(IngredientRef::Dynamic).collect();
            shapeless(&ingredients, count, inventory)
                .then(|| plain_stack(&result.item_id, result.count))?
        }
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingTransmute {
            input,
            material,
            material_count,
            add_material_count_to_result,
            result,
            ..
        }) => transmute(
            input,
            material,
            *material_count,
            *add_material_count_to_result,
            result,
            inventory,
        )?,
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingDecoratedPot { .. }) => {
            decorated_pot(count, inventory)?
        }
        GenericRecipe::Vanilla(recipe) => return special_recipes::assemble(recipe, inventory),
    };
    Some(RecipeResult {
        stack,
        remaining_items: default_remainders(inventory),
    })
}

struct Shape {
    width: usize,
    height: usize,
    left: usize,
    top: usize,
}

fn shaped(
    pattern: &[&str],
    keys: &[(char, IngredientRef<'_>)],
    shape: &Shape,
    count: usize,
    inventory: &dyn RecipeInputInventory,
) -> bool {
    // ShapedRecipe.matches tries both orientations of the positioned input.
    if pattern.len() != shape.height
        || pattern.first().map_or(0, |r| r.len()) != shape.width
        || count
            != pattern
                .iter()
                .flat_map(|r| r.chars())
                .filter(|c| *c != ' ')
                .count()
    {
        return false;
    }
    [false, true].into_iter().any(|mirror| {
        pattern.iter().enumerate().all(|(y, row)| {
            row.chars().enumerate().all(|(x, c)| {
                let x = if mirror { shape.width - 1 - x } else { x };
                let stack =
                    inventory.get_stack((y + shape.top) * inventory.get_width() + x + shape.left);
                if c == ' ' {
                    stack.is_empty()
                } else {
                    keys.iter()
                        .find(|(key, _)| *key == c)
                        .is_some_and(|(_, ingredient)| ingredient.match_item(stack.item))
                }
            })
        })
    })
}

fn shapeless(
    ingredients: &[IngredientRef<'_>],
    count: usize,
    inventory: &dyn RecipeInputInventory,
) -> bool {
    if count != ingredients.len() {
        return false;
    }
    let stacks: Vec<_> = (0..inventory.size())
        .map(|i| inventory.get_stack(i))
        .filter(|s| !s.is_empty())
        .collect();
    assign_ingredients(&stacks, ingredients, &mut vec![false; ingredients.len()], 0)
}

fn assign_ingredients(
    stacks: &[ItemStack],
    ingredients: &[IngredientRef<'_>],
    used: &mut [bool],
    slot: usize,
) -> bool {
    // ShapelessRecipe.matches uses a complete ingredient assignment, including overlapping tags.
    if slot == stacks.len() {
        return true;
    }
    for (i, ingredient) in ingredients.iter().enumerate() {
        if !used[i] && ingredient.match_item(stacks[slot].item) {
            used[i] = true;
            if assign_ingredients(stacks, ingredients, used, slot + 1) {
                return true;
            }
            used[i] = false;
        }
    }
    false
}

fn transmute(
    input: &RecipeIngredientTypes,
    material: &RecipeIngredientTypes,
    bounds: (u8, u8),
    add_count: bool,
    result: &RecipeResultStruct,
    inventory: &dyn RecipeInputInventory,
) -> Option<ItemStack> {
    // TransmuteRecipe.matches gives the input predicate precedence over material.
    let mut original = None;
    let mut materials = 0;
    for i in 0..inventory.size() {
        let stack = inventory.get_stack(i);
        if stack.is_empty() {
            continue;
        }
        if input.match_item(stack.item) {
            if original.is_some() {
                return None;
            }
            original = Some(stack);
        } else if material.match_item(stack.item) {
            materials += 1;
        } else {
            return None;
        }
    }
    let original = original?;
    if !(bounds.0..=bounds.1).contains(&materials) {
        return None;
    }
    let stack = result.assemble(Some(&original), if add_count { materials } else { 0 });
    if stack.is_empty()
        || (stack.item_count == 1 && stack.are_items_and_components_equal(&original))
    {
        return None;
    }
    Some(stack)
}

fn plain_stack(id: &str, count: u8) -> ItemStack {
    ItemStack::new(
        count,
        Item::from_registry_key(id.strip_prefix("minecraft:").unwrap_or(id)).unwrap_or(&Item::AIR),
    )
}

pub(super) fn default_remainders(inventory: &dyn RecipeInputInventory) -> Vec<ItemStack> {
    (0..inventory.size())
        .map(|i| {
            // CraftingRecipe.defaultCraftingReminder -> ItemStack.getItem resolves empties to AIR.
            let stack = inventory.get_stack(i);
            if stack.is_empty() {
                ItemStack::EMPTY.clone()
            } else {
                crafting_remainder(stack.item)
            }
        })
        .collect()
}

fn crafting_remainder(item: &Item) -> ItemStack {
    // CraftingRecipe.defaultCraftingReminder reads Item.getCraftingRemainder, not USE_REMAINDER.
    // Items.java:1940,1945,1956,2405,2730 hardcodes these five craftRemainder registrations.
    let remainder = if [
        Item::WATER_BUCKET.id,
        Item::LAVA_BUCKET.id,
        Item::MILK_BUCKET.id,
    ]
    .contains(&item.id)
    {
        &Item::BUCKET
    } else if [Item::DRAGON_BREATH.id, Item::HONEY_BOTTLE.id].contains(&item.id) {
        &Item::GLASS_BOTTLE
    } else {
        return ItemStack::EMPTY.clone();
    };
    ItemStack::new(1, remainder)
}

fn decorated_pot(count: usize, inventory: &dyn RecipeInputInventory) -> Option<ItemStack> {
    if count != 4
        || inventory.get_width() != 3
        || inventory.get_height() != 3
        || !(1..=7).step_by(2).all(|i| {
            inventory
                .get_stack(i)
                .item
                .has_tag(&tag::Item::MINECRAFT_DECORATED_POT_INGREDIENTS)
        })
    {
        return None;
    }
    Some(ItemStack::new(1, &Item::DECORATED_POT))
}
