use super::{
    Cow, CraftingRecipeTypes, Item, ItemStack, ItemStackSerializer, JavaMinecraftVersion,
    NetworkWriteExt, RECIPE_DISPLAY_SHAPELESS, RecipeIngredientTypes, RecipeResultStruct,
    SLOT_DISPLAY_COMPOSITE, SLOT_DISPLAY_ITEM_STACK, VarInt, Write, WritingError,
    crafting_category, resolve_item_tag, write_ingredient_holderset, write_ingredient_slot_display,
    write_item_slot_display, write_optional_var_int,
};
use pumpkin_data::data_component::DataComponent;

// SlotDisplays.bootstrap registry order, alongside the existing item/composite IDs.
const SLOT_DISPLAY_ONLY_WITH_COMPONENT: i32 = 3;
const SLOT_DISPLAY_DYED: i32 = 7;
const SLOT_DISPLAY_TAG: i32 = 6;

#[cfg(test)]
#[path = "recipe_book_variant_tests.rs"]
mod tests;

pub(super) fn write_entry(
    write: &mut impl Write,
    metadata: (i32, JavaMinecraftVersion, Option<i32>, u8),
    recipe: &CraftingRecipeTypes,
    material_count: u8,
    station: &Item,
) -> Result<(), WritingError> {
    let (id, version, group, flags) = metadata;
    write.write_var_int(&VarInt(id))?;
    write.write_var_int(&VarInt(RECIPE_DISPLAY_SHAPELESS))?;
    let (category, requirements) = match recipe {
        CraftingRecipeTypes::CraftingTransmute {
            category,
            input,
            material,
            result,
            add_material_count_to_result,
            ..
        } => {
            // TransmuteRecipe.display / resultDisplay enumerate each material-count variant.
            write.write_var_int(&VarInt(i32::from(material_count) + 1))?;
            write_ingredient(write, input, version)?;
            for _ in 0..material_count {
                write_ingredient(write, material, version)?;
            }
            let extra = if *add_material_count_to_result {
                material_count
            } else {
                0
            };
            result_display(write, input, result, extra, version)?;
            let mut requirements = vec![input];
            requirements.extend(std::iter::repeat_n(material, usize::from(material_count)));
            (category, requirements)
        }
        CraftingRecipeTypes::Dye {
            category,
            ingredients,
            ..
        } => {
            // DyeRecipe.display uses OnlyWithComponent and DyedSlotDemo, not a static output.
            let [target, dye] = ingredients else {
                return Err(WritingError::Message("Invalid dye ingredients".into()));
            };
            write.write_var_int(&VarInt(2))?;
            write_ingredient(write, target, version)?;
            write_dye(write, dye, version)?;
            write.write_var_int(&VarInt(SLOT_DISPLAY_DYED))?;
            write_dye(write, dye, version)?;
            write_ingredient(write, target, version)?;
            (category, vec![target, dye])
        }
        _ => return Err(WritingError::Message("Invalid variant recipe".into())),
    };
    write_item_slot_display(write, station, version)?;
    write_optional_var_int(write, group)?;
    write.write_var_int(&VarInt(crafting_category(category)))?;
    // RecipeManager.finalizeRecipeLoading shares placementInfo across all display variants.
    // TransmuteRecipe.createPlacementInfo uses the maximum material count for every variant.
    let placement = match recipe {
        CraftingRecipeTypes::CraftingTransmute {
            input,
            material,
            material_count,
            ..
        } => {
            let mut ingredients = vec![input];
            ingredients.extend(std::iter::repeat_n(material, usize::from(material_count.1)));
            ingredients
        }
        _ => requirements,
    };
    write.write_bool(true)?;
    write.write_var_int(&VarInt(placement.len() as i32))?;
    for ingredient in placement {
        write_holders(write, ingredient, version)?;
    }
    write.write_u8(flags)
}

fn result_display(
    write: &mut impl Write,
    input: &RecipeIngredientTypes,
    result: &RecipeResultStruct,
    extra: u8,
    version: JavaMinecraftVersion,
) -> Result<(), WritingError> {
    // TransmuteRecipe.resultDisplay resolves inherited item types into a composite of templates.
    if result.id.is_empty() {
        let items = ingredient_items(input, version);
        write.write_var_int(&VarInt(SLOT_DISPLAY_COMPOSITE as i32))?;
        write.write_var_int(&VarInt(items.len() as i32))?;
        for item in items {
            write_template(
                write,
                &result.assemble(Some(&ItemStack::new(1, item)), extra),
                version,
            )?;
        }
        Ok(())
    } else {
        write_template(write, &result.assemble(None, extra), version)
    }
}

fn write_dye(
    write: &mut impl Write,
    dye: &RecipeIngredientTypes,
    version: JavaMinecraftVersion,
) -> Result<(), WritingError> {
    write.write_var_int(&VarInt(SLOT_DISPLAY_ONLY_WITH_COMPONENT))?;
    write_ingredient(write, dye, version)?;
    write.write_var_int(&VarInt(i32::from(DataComponent::Dye.to_id())))
}

fn write_template(
    write: &mut impl Write,
    stack: &ItemStack,
    version: JavaMinecraftVersion,
) -> Result<(), WritingError> {
    write.write_var_int(&VarInt(SLOT_DISPLAY_ITEM_STACK as i32))?;
    ItemStackSerializer(Cow::Borrowed(stack)).write_template_with_version(write, &version)
}

fn ingredient_items(
    ingredient: &RecipeIngredientTypes,
    version: JavaMinecraftVersion,
) -> Vec<&'static Item> {
    match ingredient {
        RecipeIngredientTypes::Tagged(tag) => resolve_item_tag(tag, version).unwrap_or_default(),
        RecipeIngredientTypes::Simple(item) => {
            Item::from_registry_key(item.strip_prefix("minecraft:").unwrap_or(item))
                .into_iter()
                .collect()
        }
        RecipeIngredientTypes::OneOf(items) => items
            .iter()
            .filter_map(|item| {
                Item::from_registry_key(item.strip_prefix("minecraft:").unwrap_or(item))
            })
            .collect(),
    }
}

fn write_ingredient(
    write: &mut impl Write,
    ingredient: &RecipeIngredientTypes,
    version: JavaMinecraftVersion,
) -> Result<(), WritingError> {
    if version >= JavaMinecraftVersion::V_26_3 {
        // Ingredient.display -> SlotDisplay.TagSlotDisplay.STREAM_CODEC.
        write.write_var_int(&VarInt(SLOT_DISPLAY_TAG))?;
        write_holders(write, ingredient, version)
    } else {
        write_ingredient_slot_display(write, ingredient, version)
    }
}

fn write_holders(
    write: &mut impl Write,
    ingredient: &RecipeIngredientTypes,
    version: JavaMinecraftVersion,
) -> Result<(), WritingError> {
    if let RecipeIngredientTypes::Tagged(tag) = ingredient {
        write.write_var_int(&VarInt(0))?;
        let tag = tag.strip_prefix('#').unwrap_or(tag);
        write.write_string(&if tag.contains(':') {
            tag.to_owned()
        } else {
            format!("minecraft:{tag}")
        })
    } else {
        write_ingredient_holderset(write, ingredient, version)
    }
}
