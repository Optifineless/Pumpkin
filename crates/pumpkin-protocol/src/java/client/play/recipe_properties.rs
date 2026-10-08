use super::recipe_book_add::{
    SLOT_DISPLAY_ITEM_STACK, resolve_item_tag, write_ingredient_holderset,
};
use crate::{
    WritingError,
    codec::{item_stack_seralizer::ItemStackSerializer, var_int::VarInt},
    ser::NetworkWriteExt,
};
use pumpkin_data::{
    item::Item,
    potion_brewing::BREWING_RECIPES,
    recipes::{
        CookingRecipeType, RECIPES_COOKING, RECIPES_SMITHING_TRANSFORM, RECIPES_SMITHING_TRIM,
        RECIPES_STONECUTTING, RecipeIngredientTypes, StonecutterRecipe,
    },
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

pub(super) fn encode(version: JavaMinecraftVersion) -> Result<Vec<u8>, WritingError> {
    // ClientboundUpdateRecipesPacket.STREAM_CODEC and RecipeManager.finalizeRecipeLoading.
    let mut bytes = Vec::new();
    let properties = property_sets(version);
    bytes.write_var_int(&VarInt(properties.len() as i32))?;
    for (name, items) in properties {
        bytes.write_string(&format!("minecraft:{name}"))?;
        bytes.write_var_int(&VarInt(items.len() as i32))?;
        for id in items {
            bytes.write_var_int(&VarInt(i32::from(id)))?;
        }
    }
    bytes.write_var_int(&VarInt(RECIPES_STONECUTTING.len() as i32))?;
    for recipe in RECIPES_STONECUTTING {
        write_stonecutter_entry(&mut bytes, recipe, version)?;
    }
    Ok(bytes)
}

fn property_sets(version: JavaMinecraftVersion) -> BTreeMap<&'static str, BTreeSet<u16>> {
    // RecipePropertySet.registerVanilla keys; membership comes from recipe ingredients.
    let mut sets: BTreeMap<_, BTreeSet<_>> = [
        "smithing_base",
        "smithing_template",
        "smithing_addition",
        "furnace_input",
        "blast_furnace_input",
        "smoker_input",
        "campfire_input",
        "brewing_input",
        "brewing_reagent",
    ]
    .into_iter()
    .map(|name| (name, BTreeSet::new()))
    .collect();
    for (index, name) in [
        (0, "smithing_template"),
        (1, "smithing_base"),
        (2, "smithing_addition"),
    ] {
        for recipe in RECIPES_SMITHING_TRANSFORM {
            sets.entry(name).or_default().extend(ingredient_items(
                [&recipe.template, &recipe.base, &recipe.addition][index],
                version,
            ));
        }
        for recipe in RECIPES_SMITHING_TRIM {
            sets.entry(name).or_default().extend(ingredient_items(
                [&recipe.template, &recipe.base, &recipe.addition][index],
                version,
            ));
        }
    }
    for recipe in RECIPES_COOKING {
        let (name, recipe) = match recipe {
            CookingRecipeType::Smelting(r) => ("furnace_input", r),
            CookingRecipeType::Blasting(r) => ("blast_furnace_input", r),
            CookingRecipeType::Smoking(r) => ("smoker_input", r),
            CookingRecipeType::CampfireCooking(r) => ("campfire_input", r),
        };
        sets.entry(name)
            .or_default()
            .extend(ingredient_items(&recipe.ingredient, version));
    }
    for recipe in &BREWING_RECIPES {
        sets.entry("brewing_input")
            .or_default()
            .insert(recipe.from_item.id);
        sets.entry("brewing_reagent")
            .or_default()
            .insert(recipe.ingredient.id);
    }
    sets
}

fn ingredient_items(ingredient: &RecipeIngredientTypes, version: JavaMinecraftVersion) -> Vec<u16> {
    match ingredient {
        RecipeIngredientTypes::Tagged(tag) => resolve_item_tag(tag, version)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.id)
            .collect(),
        RecipeIngredientTypes::Simple(id) => item_id(id).into_iter().collect(),
        RecipeIngredientTypes::OneOf(ids) => ids.iter().filter_map(|id| item_id(id)).collect(),
    }
}

fn item_id(id: &str) -> Option<u16> {
    Item::from_registry_key(id.strip_prefix("minecraft:").unwrap_or(id)).map(|item| item.id)
}

fn write_stonecutter_entry(
    bytes: &mut Vec<u8>,
    recipe: &StonecutterRecipe,
    version: JavaMinecraftVersion,
) -> Result<(), WritingError> {
    // SelectableRecipe.SingleInputEntry.noRecipeCodec: ingredient then result SlotDisplay.
    write_ingredient_holderset(bytes, &recipe.ingredient, version)?;
    bytes.write_var_int(&VarInt(SLOT_DISPLAY_ITEM_STACK as i32))?;
    ItemStackSerializer(Cow::Owned(recipe.result.assemble(None, 0)))
        .write_template_with_version(bytes, &version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::recipes::RecipeResultStruct;

    #[test]
    fn stonecutter_choice_uses_vanilla_holder_set_and_template_bytes() {
        let recipe = StonecutterRecipe {
            group: None,
            ingredient: RecipeIngredientTypes::Simple("minecraft:stone"),
            result: RecipeResultStruct {
                id: "minecraft:stone",
                count: 2,
                components: None,
            },
        };
        let mut bytes = Vec::new();
        write_stonecutter_entry(&mut bytes, &recipe, JavaMinecraftVersion::V_26_3).unwrap();
        // HolderSet size+1, stone holder, item-stack display, stone, count, empty patch.
        assert_eq!(bytes, [2, 1, 5, 1, 2, 0, 0]);
    }
}
