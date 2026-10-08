use super::*;
use crate::ser::NetworkReadExt;
use pumpkin_data::recipes::{RECIPES_CRAFTING, RecipeCategoryTypes};

#[test]
fn map_cloning_material_variants_and_inherited_templates_match_wire_fixtures() {
    let recipe = RECIPES_CRAFTING
        .iter()
        .find(|recipe| {
            matches!(
                recipe,
                CraftingRecipeTypes::CraftingTransmute {
                    group: Some("map_cloning"),
                    ..
                }
            )
        })
        .unwrap();
    let variants: Vec<_> = pumpkin_data::crafting_displays()
        .filter(|(entry, _)| std::ptr::eq(*entry, recipe))
        .map(|(_, count)| count)
        .collect();
    assert_eq!(variants, [1, 2, 3, 4, 5, 6, 7, 8]);
    for (count, expected) in [
        (
            1,
            include_bytes!("../../../../tests/fixtures/recipe_displays/map-cloning-1.bin")
                .as_slice(),
        ),
        (
            8,
            include_bytes!("../../../../tests/fixtures/recipe_displays/map-cloning-8.bin")
                .as_slice(),
        ),
    ] {
        let mut bytes = Vec::new();
        write_entry(
            &mut bytes,
            (77, JavaMinecraftVersion::V_26_3, Some(0), 3),
            recipe,
            count,
            &Item::CRAFTING_TABLE,
        )
        .unwrap();
        assert_eq!(bytes, expected);
    }
}

#[test]
fn dyed_armor_preview_group_and_category_match_wire_fixture() {
    let recipe = RECIPES_CRAFTING
        .iter()
        .find(|recipe| {
            matches!(recipe,
        CraftingRecipeTypes::Dye { result, .. } if result.id == "minecraft:leather_helmet")
        })
        .unwrap();
    assert!(matches!(
        recipe,
        CraftingRecipeTypes::Dye {
            group: Some("dyed_armor"),
            category: RecipeCategoryTypes::Misc,
            ..
        }
    ));
    assert!(pumpkin_data::crafting_displays().any(|(entry, _)| std::ptr::eq(entry, recipe)));
    let mut bytes = Vec::new();
    write_entry(
        &mut bytes,
        (77, JavaMinecraftVersion::V_26_3, Some(0), 3),
        recipe,
        0,
        &Item::CRAFTING_TABLE,
    )
    .unwrap();
    assert_eq!(
        bytes,
        include_bytes!("../../../../tests/fixtures/recipe_displays/leather-helmet-dyed.bin")
    );
}

#[test]
fn transmute_display_applies_inherited_component_template() {
    let recipe = CraftingRecipeTypes::CraftingTransmute {
        category: RecipeCategoryTypes::Misc,
        group: None,
        input: RecipeIngredientTypes::Simple("minecraft:stone"),
        material: RecipeIngredientTypes::Simple("minecraft:paper"),
        material_count: (1, 1),
        add_material_count_to_result: true,
        result: RecipeResultStruct {
            id: "",
            count: 1,
            components: Some(r#"{"minecraft:custom_name":{"text":"Named","bold":true}}"#),
        },
    };
    let mut bytes = Vec::new();
    write_entry(
        &mut bytes,
        (0, JavaMinecraftVersion::V_26_3, None, 0),
        &recipe,
        1,
        &Item::CRAFTING_TABLE,
    )
    .unwrap();
    let mut input = std::io::Cursor::new(bytes);
    for _ in 0..9 {
        input.get_var_int().unwrap();
    }
    assert_eq!(
        input.get_var_int().unwrap().0,
        SLOT_DISPLAY_COMPOSITE as i32
    );
    assert_eq!(input.get_var_int().unwrap().0, 1);
    assert_eq!(
        input.get_var_int().unwrap().0,
        SLOT_DISPLAY_ITEM_STACK as i32
    );
    let stack =
        ItemStackSerializer::read_template_with_version(&mut input, &JavaMinecraftVersion::V_26_3)
            .unwrap();
    assert_eq!(stack.0.item_count, 2);
    assert_eq!(
        stack
            .0
            .get_data_component::<pumpkin_data::data_component_impl::CustomNameImpl>()
            .unwrap()
            .name,
        pumpkin_util::text::TextComponent::text("Named").bold()
    );
}
