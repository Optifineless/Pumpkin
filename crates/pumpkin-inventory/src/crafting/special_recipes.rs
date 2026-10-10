use super::{
    crafting_screen_handler::RecipeResult, recipe_matching::default_remainders,
    recipes::RecipeInputInventory,
};
use pumpkin_data::{
    data_component_impl::{
        BannerPatternsImpl, DamageImpl, DyeImpl, DyedColorImpl, EnchantmentsImpl,
        FireworkExplosionImpl, FireworkExplosionShape, FireworksImpl, MaxDamageImpl,
        WrittenBookContentImpl,
    },
    item_stack::ItemStack,
    recipes::{CraftingRecipeTypes, RecipeIngredientTypes, RecipeResultStruct},
    tag::{self, Taggable},
};
use pumpkin_util::text::color::ARGBColor;
use std::borrow::Cow;

pub(super) fn assemble(
    recipe: &CraftingRecipeTypes,
    inventory: &dyn RecipeInputInventory,
) -> Option<RecipeResult> {
    let stacks: Vec<_> = (0..inventory.size())
        .map(|i| (i, inventory.get_stack(i)))
        .filter(|(_, s)| !s.is_empty())
        .collect();
    let mut remaining_items = default_remainders(inventory);
    let stack = match recipe {
        CraftingRecipeTypes::FireworkStar {
            shapes,
            trail,
            twinkle,
            fuel,
            dye,
            result,
        } => firework_star(&stacks, shapes, trail, twinkle, fuel, dye, result)?,
        CraftingRecipeTypes::FireworkStarFade {
            target,
            dye,
            result,
        } => firework_star_fade(&stacks, target, dye, result)?,
        CraftingRecipeTypes::FireworkRocket {
            ingredients,
            result,
        } => rocket(&stacks, ingredients, result)?,
        CraftingRecipeTypes::RepairItem => repair(&stacks)?,
        CraftingRecipeTypes::Dye {
            ingredients,
            result,
            ..
        } => dye(&stacks, ingredients, result)?,
        CraftingRecipeTypes::BannerDuplicate {
            ingredients,
            result,
        } => {
            let (slot, source) = banner(&stacks, ingredients)?;
            if remaining_items[slot].is_empty() {
                remaining_items[slot] = source.copy_with_count(1);
            }
            result.assemble(Some(source), 0)
        }
        CraftingRecipeTypes::BookCloning {
            ingredients,
            allowed_generations,
            result,
        } => {
            let (_, source, copies) = book(&stacks, ingredients, *allowed_generations)?;
            // BookCloningRecipe.getRemainingItems stops after the first written-book remainder.
            for i in 0..remaining_items.len() {
                let input = inventory.get_stack(i);
                if remaining_items[i].is_empty()
                    && input
                        .get_data_component::<WrittenBookContentImpl>()
                        .is_some()
                {
                    remaining_items[i] = input.copy_with_count(1);
                    remaining_items[i + 1..].fill(ItemStack::EMPTY.clone());
                    break;
                }
            }
            let mut stack = result.assemble(Some(source), copies - 1);
            let mut content = source
                .get_data_component::<WrittenBookContentImpl>()?
                .clone();
            content.generation += 1;
            stack.set_data_component(content);
            stack
        }
        _ => return None,
    };
    Some(RecipeResult {
        stack,
        remaining_items,
    })
}

fn firework_star(
    stacks: &[(usize, ItemStack)],
    shapes: &[(&str, RecipeIngredientTypes)],
    trail: &RecipeIngredientTypes,
    twinkle: &RecipeIngredientTypes,
    fuel: &RecipeIngredientTypes,
    dye: &RecipeIngredientTypes,
    result: &RecipeResultStruct,
) -> Option<ItemStack> {
    if !firework_star_matches(stacks, shapes, trail, twinkle, fuel, dye) {
        return None;
    }

    let mut has_trail = false;
    let mut has_twinkle = false;
    let mut shape = FireworkExplosionShape::SmallBall;
    let mut colors = Vec::new();
    for (_, stack) in stacks {
        // FireworkStarRecipe.assemble checks shapes before effects, unlike matches.
        if let Some(matched_shape) = firework_star_shape(stack, shapes) {
            shape = matched_shape;
        } else if twinkle.match_item(stack.item) {
            has_twinkle = true;
        } else if trail.match_item(stack.item) {
            has_trail = true;
        } else if dye.match_item(stack.item)
            && let Some(dye_component) = stack.get_data_component::<DyeImpl>()
        {
            colors.push(dye_component.color.firework_color() as i32);
        }
    }

    let mut stack = result.assemble(None, 0);
    stack.set_data_component(FireworkExplosionImpl::new(
        shape,
        colors,
        Vec::new(),
        has_trail,
        has_twinkle,
    ));
    Some(stack)
}

// FireworkStarRecipe.matches checks effects before shapes; overlapping ingredients count once.
fn firework_star_matches(
    stacks: &[(usize, ItemStack)],
    shapes: &[(&str, RecipeIngredientTypes)],
    trail: &RecipeIngredientTypes,
    twinkle: &RecipeIngredientTypes,
    fuel: &RecipeIngredientTypes,
    dye: &RecipeIngredientTypes,
) -> bool {
    let mut has_fuel = false;
    let mut has_dye = false;
    let mut has_shape = false;
    let mut has_trail = false;
    let mut has_twinkle = false;

    for (_, stack) in stacks {
        if twinkle.match_item(stack.item) {
            if has_twinkle {
                return false;
            }
            has_twinkle = true;
        } else if trail.match_item(stack.item) {
            if has_trail {
                return false;
            }
            has_trail = true;
        } else if fuel.match_item(stack.item) {
            if has_fuel {
                return false;
            }
            has_fuel = true;
        } else if dye.match_item(stack.item) && stack.get_data_component::<DyeImpl>().is_some() {
            has_dye = true;
        } else {
            if firework_star_shape(stack, shapes).is_none() {
                return false;
            }
            if has_shape {
                return false;
            }
            has_shape = true;
        }
    }

    has_fuel && has_dye
}

fn firework_star_shape(
    stack: &ItemStack,
    shapes: &[(&str, RecipeIngredientTypes)],
) -> Option<FireworkExplosionShape> {
    shapes.iter().find_map(|(name, ingredient)| {
        ingredient
            .match_item(stack.item)
            .then(|| FireworkExplosionShape::from_name(name))
            .flatten()
    })
}

fn firework_star_fade(
    stacks: &[(usize, ItemStack)],
    target: &RecipeIngredientTypes,
    dye: &RecipeIngredientTypes,
    result: &RecipeResultStruct,
) -> Option<ItemStack> {
    // FireworkStarFadeRecipe.transmutes the target and replaces only its fade colors.
    let mut target_stack = None;
    let mut fade_colors = Vec::new();
    for (_, stack) in stacks {
        if dye.match_item(stack.item) {
            let dye_component = stack.get_data_component::<DyeImpl>()?;
            fade_colors.push(dye_component.color.firework_color() as i32);
        } else if target.match_item(stack.item) {
            if target_stack.is_some() {
                return None;
            }
            target_stack = Some(stack);
        } else {
            return None;
        }
    }

    let target_stack = target_stack?;
    if fade_colors.is_empty() {
        return None;
    }

    let mut stack = result.assemble(Some(target_stack), 0);
    let mut explosion = stack
        .get_data_component::<FireworkExplosionImpl>()
        .cloned()
        .unwrap_or_else(|| {
            // FireworkExplosion.DEFAULT is a small ball with no colors or effects.
            FireworkExplosionImpl::new(
                FireworkExplosionShape::SmallBall,
                Vec::new(),
                Vec::new(),
                false,
                false,
            )
        });
    explosion.fade_colors = fade_colors;
    stack.set_data_component(explosion);
    Some(stack)
}

fn rocket(
    stacks: &[(usize, ItemStack)],
    ingredients: &[RecipeIngredientTypes],
    result: &RecipeResultStruct,
) -> Option<ItemStack> {
    // FireworkRocketRecipe.matches / assemble: one shell, 1..3 fuel slots, optional stars.
    let [shell, fuel, star] = ingredients else {
        return None;
    };
    let mut shell_count = 0;
    let mut fuel_count = 0;
    let mut explosions = Vec::new();
    for (_, stack) in stacks {
        if shell.match_item(stack.item) {
            shell_count += 1;
        } else if fuel.match_item(stack.item) {
            fuel_count += 1;
        } else if star.match_item(stack.item) {
            if let Some(explosion) = stack.get_data_component::<FireworkExplosionImpl>() {
                explosions.push(explosion.clone());
            }
        } else {
            return None;
        }
    }
    if shell_count != 1 || !(1..=3).contains(&fuel_count) {
        return None;
    }
    // FireworkRocketRecipe.assemble applies the recipe template after its fireworks patch.
    let mut patch = ItemStack::new(1, &pumpkin_data::item::Item::FIREWORK_ROCKET);
    patch.set_data_component(FireworksImpl::new(fuel_count, explosions));
    Some(result.assemble(Some(&patch), 0))
}

fn repair(stacks: &[(usize, ItemStack)]) -> Option<ItemStack> {
    // RepairItemRecipe.canCombine / assemble creates a new stack and retains only curses.
    let [(_, first), (_, second)] = stacks else {
        return None;
    };
    if first.item != second.item
        || first.item_count != 1
        || second.item_count != 1
        || first.get_data_component::<DamageImpl>().is_none()
        || second.get_data_component::<DamageImpl>().is_none()
    {
        return None;
    }
    let max_first = first.get_max_damage()?;
    let max_second = second.get_max_damage()?;
    let durability = max_first.max(max_second);
    let remaining =
        max_first - first.get_damage() + max_second - second.get_damage() + durability * 5 / 100;
    let mut stack = ItemStack::new(1, first.item);
    stack.set_data_component(MaxDamageImpl {
        max_damage: durability,
    });
    stack.set_damage((durability - remaining).max(0));
    let mut curses = Vec::new();
    for source in [first, second] {
        for (enchantment, level) in
            crate::grindstone_screen_handler::get_enchantments_for_crafting(source)
        {
            if enchantment.has_tag(&tag::Enchantment::MINECRAFT_CURSE) {
                if let Some((_, stored)) = curses.iter_mut().find(|(e, _)| *e == enchantment) {
                    *stored = level.max(*stored);
                } else {
                    curses.push((enchantment, level));
                }
            }
        }
    }
    stack.set_data_component(EnchantmentsImpl {
        enchantment: Cow::Owned(curses),
    });
    Some(stack)
}

fn dye(
    stacks: &[(usize, ItemStack)],
    ingredients: &[RecipeIngredientTypes],
    result: &RecipeResultStruct,
) -> Option<ItemStack> {
    // DyeRecipe.matches / assemble and DyedItemColor.applyDyes use component colors.
    let [target, dye] = ingredients else {
        return None;
    };
    let mut source = None;
    let mut colors = Vec::new();
    for (_, stack) in stacks {
        if target.match_item(stack.item) {
            if source.is_some() {
                return None;
            }
            source = Some(stack);
        } else if dye.match_item(stack.item) {
            colors.push(
                stack
                    .get_data_component::<DyeImpl>()?
                    .color
                    .texture_diffuse_color(),
            );
        } else {
            return None;
        }
    }
    let source = source?;
    if colors.is_empty() {
        return None;
    }
    if let Some(color) = source.get_data_component::<DyedColorImpl>() {
        colors.push(color.rgb as u32);
    }
    let mut channels = [0u32; 3];
    let mut intensity = 0;
    for color in &colors {
        let color = ARGBColor::from_argb_u32(*color);
        let rgb = [
            u32::from(color.red),
            u32::from(color.green),
            u32::from(color.blue),
        ];
        intensity += rgb.into_iter().max().unwrap_or_default();
        for (total, channel) in channels.iter_mut().zip(rgb) {
            *total += channel;
        }
    }
    for channel in &mut channels {
        *channel /= colors.len() as u32;
    }
    let average = intensity as f32 / colors.len() as f32;
    let peak = channels.into_iter().max().unwrap_or_default() as f32;
    let [r, g, b] = channels.map(|v| (v as f32 * average / peak) as u8);
    let mut stack = result.assemble(Some(source), 0);
    stack.set_data_component(DyedColorImpl {
        rgb: ARGBColor::new(0, r, g, b).to_argb_int(),
    });
    Some(stack)
}

fn banner<'a>(
    stacks: &'a [(usize, ItemStack)],
    ingredients: &[RecipeIngredientTypes],
) -> Option<(usize, &'a ItemStack)> {
    // BannerDuplicateRecipe.matches checks the same base color and one patterned source.
    let [(_, first), (_, second)] = stacks else {
        return None;
    };
    if first.item != second.item || !ingredients.first()?.match_item(first.item) {
        return None;
    }
    let mut source = None;
    for (slot, stack) in stacks {
        let count = stack
            .get_data_component::<BannerPatternsImpl>()
            .map_or(0, |p| p.layers.len());
        if count > 6 {
            return None;
        }
        if count > 0 {
            if source.is_some() {
                return None;
            }
            source = Some((*slot, stack));
        }
    }
    source
}

fn book<'a>(
    stacks: &'a [(usize, ItemStack)],
    ingredients: &[RecipeIngredientTypes],
    generations: (u8, u8),
) -> Option<(usize, &'a ItemStack, u8)> {
    // BookCloningRecipe.matches retains the original and bounds the copy generation.
    let [source, material] = ingredients else {
        return None;
    };
    let mut original = None;
    let mut copies = 0;
    for (slot, stack) in stacks {
        if source.match_item(stack.item) {
            if original.is_some() {
                return None;
            }
            let content = stack.get_data_component::<WrittenBookContentImpl>()?;
            if !(i32::from(generations.0)..=i32::from(generations.1)).contains(&content.generation)
            {
                return None;
            }
            original = Some((*slot, stack));
        } else if material.match_item(stack.item) {
            copies += 1;
        } else {
            return None;
        }
    }
    let (slot, source) = original?;
    (copies > 0).then_some((slot, source, copies))
}

#[cfg(test)]
#[path = "special_recipes_tests.rs"]
mod tests;
