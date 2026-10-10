use super::*;
use crate::{crafting::crafting_inventory::CraftingInventory, inventory::Inventory};
use pumpkin_data::{
    data_component_impl::{DyeImpl, FireworkExplosionShape},
    dye_color::DyeColor,
    item::Item,
    recipes::{RECIPES_CRAFTING, RecipeIngredientTypes, RecipeResultStruct},
};

static OVERLAPPING_SHAPES: [(&str, RecipeIngredientTypes); 1] = [(
    "large_ball",
    RecipeIngredientTypes::Simple("minecraft:glowstone_dust"),
)];

fn recipe(
    predicate: impl Fn(&CraftingRecipeTypes) -> bool,
) -> Option<&'static CraftingRecipeTypes> {
    RECIPES_CRAFTING.iter().find(|recipe| predicate(recipe))
}

fn stack(item: &'static Item) -> ItemStack {
    ItemStack::new(1, item)
}

fn dye(item: &'static Item, color: DyeColor) -> ItemStack {
    let mut stack = stack(item);
    stack.set_data_component(DyeImpl { color });
    stack
}

fn craft(
    recipe: &CraftingRecipeTypes,
    ingredients: &[ItemStack],
    grid_width: u8,
) -> Option<RecipeResult> {
    let inventory = CraftingInventory::new(grid_width, grid_width);
    for (slot, ingredient) in ingredients.iter().enumerate() {
        inventory.set_stack(slot, ingredient.clone());
    }
    super::assemble(recipe, &inventory)
}

#[test]
fn firework_star_uses_dye_components_in_player_crafting_grid() {
    let recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStar { .. }));
    assert!(
        recipe.is_some(),
        "generated firework star recipe is missing"
    );
    let Some(recipe) = recipe else { return };

    let result = craft(
        recipe,
        &[stack(&Item::GUNPOWDER), dye(&Item::RED_DYE, DyeColor::Red)],
        2,
    );
    assert!(result.is_some(), "gunpowder and dye should craft a star");
    let Some(result) = result else { return };
    assert_eq!(result.stack.item.id, Item::FIREWORK_STAR.id);

    let explosion = result.stack.get_data_component::<FireworkExplosionImpl>();
    assert!(explosion.is_some(), "crafted star needs explosion data");
    let Some(explosion) = explosion else { return };
    assert_eq!(explosion.shape, FireworkExplosionShape::SmallBall);
    assert_eq!(explosion.colors, [DyeColor::Red.firework_color() as i32]);
    assert!(explosion.fade_colors.is_empty());
    assert!(!explosion.has_trail);
    assert!(!explosion.has_twinkle);
}

#[test]
fn firework_star_supports_vanilla_shapes_and_effect_modifiers() {
    let recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStar { .. }));
    assert!(
        recipe.is_some(),
        "generated firework star recipe is missing"
    );
    let Some(recipe) = recipe else { return };
    let shapes = [
        (&Item::FIRE_CHARGE, FireworkExplosionShape::LargeBall),
        (&Item::GOLD_NUGGET, FireworkExplosionShape::Star),
        (&Item::CREEPER_HEAD, FireworkExplosionShape::Creeper),
        (&Item::FEATHER, FireworkExplosionShape::Burst),
    ];

    for (shape_item, expected_shape) in shapes {
        let result = craft(
            recipe,
            &[
                stack(&Item::GUNPOWDER),
                dye(&Item::RED_DYE, DyeColor::Red),
                dye(&Item::BLUE_DYE, DyeColor::Blue),
                stack(shape_item),
                stack(&Item::DIAMOND),
                stack(&Item::GLOWSTONE_DUST),
            ],
            3,
        );
        assert!(result.is_some(), "shape ingredient should craft a star");
        let Some(result) = result else { return };
        let explosion = result.stack.get_data_component::<FireworkExplosionImpl>();
        assert!(explosion.is_some(), "crafted star needs explosion data");
        let Some(explosion) = explosion else { return };
        assert_eq!(explosion.shape, expected_shape);
        assert_eq!(
            explosion.colors,
            [
                DyeColor::Red.firework_color() as i32,
                DyeColor::Blue.firework_color() as i32,
            ]
        );
        assert!(explosion.has_trail);
        assert!(explosion.has_twinkle);
    }
}

#[test]
fn firework_star_match_and_assembly_use_vanilla_overlap_priority() {
    let recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStar { .. }));
    assert!(
        recipe.is_some(),
        "generated firework star recipe is missing"
    );
    let Some(CraftingRecipeTypes::FireworkStar {
        trail,
        twinkle,
        fuel,
        dye: dye_ingredient,
        result,
        ..
    }) = recipe
    else {
        return;
    };
    let overlapping = CraftingRecipeTypes::FireworkStar {
        shapes: &OVERLAPPING_SHAPES,
        trail: trail.clone(),
        twinkle: twinkle.clone(),
        fuel: fuel.clone(),
        dye: dye_ingredient.clone(),
        result: result.clone(),
    };

    let crafted = craft(
        &overlapping,
        &[
            stack(&Item::GUNPOWDER),
            dye(&Item::RED_DYE, DyeColor::Red),
            stack(&Item::GLOWSTONE_DUST),
        ],
        2,
    );
    assert!(
        crafted.is_some(),
        "overlapping ingredient should still match"
    );
    let Some(crafted) = crafted else { return };
    let explosion = crafted.stack.get_data_component::<FireworkExplosionImpl>();
    assert!(
        explosion.is_some(),
        "overlapping shape should produce explosion data"
    );
    let Some(explosion) = explosion else { return };
    assert_eq!(explosion.shape, FireworkExplosionShape::LargeBall);
    assert!(!explosion.has_twinkle);
}

#[test]
fn firework_star_rejects_invalid_ingredient_combinations() {
    let recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStar { .. }));
    assert!(
        recipe.is_some(),
        "generated firework star recipe is missing"
    );
    let Some(recipe) = recipe else { return };
    let invalid = [
        vec![stack(&Item::GUNPOWDER)],
        vec![
            stack(&Item::GUNPOWDER),
            stack(&Item::GUNPOWDER),
            dye(&Item::RED_DYE, DyeColor::Red),
        ],
        vec![
            stack(&Item::GUNPOWDER),
            dye(&Item::RED_DYE, DyeColor::Red),
            stack(&Item::FIRE_CHARGE),
            stack(&Item::GOLD_NUGGET),
        ],
        vec![
            stack(&Item::GUNPOWDER),
            dye(&Item::RED_DYE, DyeColor::Red),
            stack(&Item::DIAMOND),
            stack(&Item::DIAMOND),
        ],
        vec![
            stack(&Item::GUNPOWDER),
            dye(&Item::RED_DYE, DyeColor::Red),
            stack(&Item::DIRT),
        ],
    ];

    for ingredients in invalid {
        assert!(craft(recipe, &ingredients, 3).is_none());
    }
}

#[test]
fn firework_star_fade_replaces_only_fade_colors() {
    let recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStarFade { .. }));
    assert!(
        recipe.is_some(),
        "generated firework star fade recipe is missing"
    );
    let Some(recipe) = recipe else { return };
    let mut star = stack(&Item::FIREWORK_STAR);
    star.set_data_component(FireworkExplosionImpl::new(
        FireworkExplosionShape::Creeper,
        vec![0x112233],
        vec![0x445566],
        true,
        true,
    ));

    let result = craft(
        recipe,
        &[
            star,
            dye(&Item::RED_DYE, DyeColor::Red),
            dye(&Item::BLUE_DYE, DyeColor::Blue),
        ],
        3,
    );
    assert!(result.is_some(), "star and dye should craft a faded star");
    let Some(result) = result else { return };
    let explosion = result.stack.get_data_component::<FireworkExplosionImpl>();
    assert!(explosion.is_some(), "faded star needs explosion data");
    let Some(explosion) = explosion else { return };
    assert_eq!(explosion.shape, FireworkExplosionShape::Creeper);
    assert_eq!(explosion.colors, [0x112233]);
    assert_eq!(
        explosion.fade_colors,
        [
            DyeColor::Red.firework_color() as i32,
            DyeColor::Blue.firework_color() as i32,
        ]
    );
    assert!(explosion.has_trail);
    assert!(explosion.has_twinkle);
}

#[test]
fn firework_star_fade_reads_the_assembled_template_explosion() {
    const COMPONENTS: &str = r#"{"minecraft:firework_explosion":{"shape":"creeper"}}"#;
    let recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStarFade { .. }));
    assert!(
        recipe.is_some(),
        "generated firework star fade recipe is missing"
    );
    let Some(CraftingRecipeTypes::FireworkStarFade {
        target,
        dye: dye_ingredient,
        result,
    }) = recipe
    else {
        return;
    };
    let fade_recipe = CraftingRecipeTypes::FireworkStarFade {
        target: target.clone(),
        dye: dye_ingredient.clone(),
        result: RecipeResultStruct {
            id: result.id,
            count: result.count,
            components: Some(COMPONENTS),
        },
    };
    let mut original = stack(&Item::FIREWORK_STAR);
    original.set_data_component(FireworkExplosionImpl::new(
        FireworkExplosionShape::LargeBall,
        vec![0x112233],
        Vec::new(),
        true,
        true,
    ));

    let crafted = craft(
        &fade_recipe,
        &[original, dye(&Item::BLUE_DYE, DyeColor::Blue)],
        2,
    );
    assert!(crafted.is_some(), "star and dye should craft a faded star");
    let Some(crafted) = crafted else { return };
    let explosion = crafted.stack.get_data_component::<FireworkExplosionImpl>();
    assert!(explosion.is_some(), "faded star should keep explosion data");
    let Some(explosion) = explosion else { return };
    assert_eq!(explosion.shape, FireworkExplosionShape::Creeper);
    assert!(explosion.colors.is_empty());
    assert_eq!(
        explosion.fade_colors,
        [DyeColor::Blue.firework_color() as i32]
    );
}

#[test]
fn crafted_firework_star_keeps_its_explosion_in_a_rocket() {
    let star_recipe = recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkStar { .. }));
    assert!(
        star_recipe.is_some(),
        "generated firework star recipe is missing"
    );
    let Some(star_recipe) = star_recipe else {
        return;
    };
    let star_result = craft(
        star_recipe,
        &[
            stack(&Item::GUNPOWDER),
            dye(&Item::RED_DYE, DyeColor::Red),
            stack(&Item::DIAMOND),
        ],
        3,
    );
    assert!(
        star_result.is_some(),
        "firework star ingredients should craft"
    );
    let Some(star_result) = star_result else {
        return;
    };
    let expected_explosion = star_result
        .stack
        .get_data_component::<FireworkExplosionImpl>()
        .cloned();
    assert!(
        expected_explosion.is_some(),
        "crafted star needs explosion data"
    );
    let Some(expected_explosion) = expected_explosion else {
        return;
    };

    let rocket_recipe =
        recipe(|recipe| matches!(recipe, CraftingRecipeTypes::FireworkRocket { .. }));
    assert!(
        rocket_recipe.is_some(),
        "generated firework rocket recipe is missing"
    );
    let Some(rocket_recipe) = rocket_recipe else {
        return;
    };
    let result = craft(
        rocket_recipe,
        &[
            stack(&Item::PAPER),
            stack(&Item::GUNPOWDER),
            stack(&Item::GUNPOWDER),
            star_result.stack,
        ],
        3,
    );
    assert!(
        result.is_some(),
        "paper, fuel and a star should craft rockets"
    );
    let Some(result) = result else { return };
    assert_eq!(result.stack.item.id, Item::FIREWORK_ROCKET.id);
    assert_eq!(result.stack.item_count, 3);
    let fireworks = result.stack.get_data_component::<FireworksImpl>();
    assert!(fireworks.is_some(), "crafted rocket needs firework data");
    let Some(fireworks) = fireworks else { return };
    assert_eq!(fireworks.flight_duration, 2);
    assert_eq!(fireworks.explosions, [expected_explosion]);

    assert!(
        craft(
            rocket_recipe,
            &[
                stack(&Item::PAPER),
                stack(&Item::GUNPOWDER),
                stack(&Item::GUNPOWDER),
                stack(&Item::GUNPOWDER),
                stack(&Item::GUNPOWDER),
            ],
            3,
        )
        .is_none()
    );
}
