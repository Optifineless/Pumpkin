use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use crate::{item::Item, item_stack::ItemStack, recipes::RecipeResultStruct};

impl RecipeResultStruct {
    /// Applies the recipe template over the original input's component patch.
    pub fn assemble(&self, original: Option<&ItemStack>, extra_count: u8) -> ItemStack {
        // TransmuteRecipe.createWithOriginalComponents / ItemStackTemplate.apply.
        let item = if self.id.is_empty() {
            original.map(|s| s.item)
        } else {
            Item::from_registry_key(self.id.strip_prefix("minecraft:").unwrap_or(self.id))
        };
        let Some(item) = item else {
            return ItemStack::EMPTY.clone();
        };
        let mut stack = ItemStack::new(self.count.saturating_add(extra_count), item);
        if let Some(original) = original {
            stack.patch.clone_from(&original.patch);
        }
        if let Some(components) = self.components
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(components)
            && let NbtTag::Compound(components) = json_tag(value)
        {
            let mut nbt = NbtCompound::new();
            stack.write_item_stack(&mut nbt);
            let mut patch = nbt.get_compound("components").cloned().unwrap_or_default();
            for (key, value) in components.child_tags {
                // DataComponentPatch.apply removes the opposite entry before overlaying it.
                let opposite = key
                    .strip_prefix('!')
                    .map_or_else(|| format!("!{key}"), str::to_owned);
                patch.child_tags.remove(opposite.as_str());
                patch.put(&key, value);
            }
            nbt.put_compound("components", patch);
            if let Some(result) = ItemStack::read_item_stack(&nbt) {
                stack = result;
            }
        }
        stack
    }
}

fn json_tag(value: serde_json::Value) -> NbtTag {
    match value {
        serde_json::Value::Null => NbtTag::End,
        serde_json::Value::Bool(v) => NbtTag::Byte(i8::from(v)),
        serde_json::Value::Number(v) => v.as_i64().map_or_else(
            || NbtTag::Double(v.as_f64().unwrap_or_default()),
            |v| i32::try_from(v).map_or(NbtTag::Long(v), NbtTag::Int),
        ),
        serde_json::Value::String(v) => NbtTag::String(v.into()),
        serde_json::Value::Array(v) => NbtTag::List(v.into_iter().map(json_tag).collect()),
        serde_json::Value::Object(v) => {
            let mut compound = NbtCompound::new();
            for (key, value) in v {
                compound.put(&key, json_tag(value));
            }
            NbtTag::Compound(compound)
        }
    }
}

impl crate::recipes::CraftingRecipeTypes {
    /// Reports whether Pumpkin can encode this recipe's static recipe-book display.
    pub fn has_static_display(&self) -> bool {
        matches!(
            self,
            Self::CraftingShaped { .. }
                | Self::CraftingShapeless { .. }
                | Self::CraftingTransmute { .. }
                | Self::Dye { .. }
        )
    }
}

/// Enumerates vanilla recipe display variants; placement and packets share these IDs.
pub fn crafting_displays()
-> impl Iterator<Item = (&'static crate::recipes::CraftingRecipeTypes, u8)> {
    use crate::recipes::CraftingRecipeTypes;
    crate::recipes::RECIPES_CRAFTING.iter().flat_map(|recipe| {
        let counts = match recipe {
            CraftingRecipeTypes::CraftingTransmute { material_count, .. } => {
                material_count.0..=material_count.1
            }
            _ if recipe.has_static_display() => 0..=0,
            _ => 1..=0,
        };
        counts.map(move |count| (recipe, count))
    })
}
