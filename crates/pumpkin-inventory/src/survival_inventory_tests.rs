use crate::{
    Inventory, SimpleInventory,
    anvil::anvil_screen_handler::AnvilScreenHandler,
    cartography_table_screen_handler::CartographyTableScreenHandler,
    crafting::{
        crafting_inventory::CraftingInventory,
        crafting_screen_handler::{CraftingTableScreenHandler, match_crafting_recipe},
    },
    ext_review_tests::Player,
    screen_handler::ScreenHandler,
    shulker_box_screen_handler::ShulkerBoxScreenHandler,
    smithing_table_screen_handler::SmithingTableScreenHandler,
    stonecutter_screen_handler::StonecutterScreenHandler,
};
use pumpkin_data::{
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, ContainerImpl, DamageImpl, DyedColorImpl,
        EnchantmentsImpl, FireworkExplosionImpl, FireworkExplosionShape, FireworksImpl, MapIdImpl,
        WrittenBookContentImpl,
    },
    dye_color::DyeColor,
    enchantment::Enchantment,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::java::server::play::SlotActionType;
use pumpkin_util::text::TextComponent;
use std::{
    borrow::Cow,
    sync::{Arc, atomic::Ordering::Relaxed},
};

#[test]
fn ctrl_q_unaffordable_anvil_result_stops_without_drops() {
    let player = Player::new(0);
    player.levels.store(0, Relaxed);
    let input = Arc::new(SimpleInventory::new(3));
    input.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
    let mut handler = AnvilScreenHandler::new(1, &player.inventory, input);
    assert!(handler.set_item_name("key", false));
    handler.on_slot_click(2, 1, SlotActionType::Throw, &player);
    assert_eq!(handler.inventory.get_stack(0).item_count, 1);
    assert!(player.drops.lock().unwrap().is_empty());
}

#[test]
fn review3_anvil_repairs_translated_name_without_rename_charge() {
    use pumpkin_data::data_component_impl::CustomNameImpl;
    use pumpkin_nbt::{NbtCompound, tag::NbtTag};
    let player = Player::new(0);
    for (key, fallback, arguments, rendered) in [
        ("container.chest", "Vault", vec![], "Chest"),
        ("unknown.key", "Key %s", vec![NbtTag::Int(1)], "Key 1"),
    ] {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", key.into());
        nbt.put_string("fallback", fallback.into());
        if !arguments.is_empty() {
            nbt.put_list("with", arguments);
        }
        let tag = NbtTag::Compound(nbt);
        let mut pickaxe = ItemStack::new(1, &Item::DIAMOND_PICKAXE);
        pickaxe.set_data_component(DamageImpl { damage: 100 });
        pickaxe.set_data_component(CustomNameImpl::read_data(&tag).unwrap());
        let inventory = Arc::new(SimpleInventory::new(3));
        inventory.set_stack(0, pickaxe);
        inventory.set_stack(1, ItemStack::new(1, &Item::DIAMOND));
        let mut menu = AnvilScreenHandler::new(1, &player.inventory, inventory.clone());
        menu.set_item_name(rendered, false);
        assert_eq!(menu.repair_cost.load(Relaxed), 1);
        let result = inventory.get_stack(2);
        assert_eq!(result.get_damage(), 0);
        assert_eq!(result.get_hover_name(), rendered);
        assert_eq!(
            result.get_data_component::<CustomNameImpl>().unwrap(),
            &CustomNameImpl::read_data(&tag).unwrap()
        );
    }
}

#[test]
fn cake_returns_three_buckets_to_the_grid() {
    let player = Player::new(0);
    let mut menu = CraftingTableScreenHandler::new(1, &player.inventory, None);
    for (i, item) in [
        &Item::MILK_BUCKET,
        &Item::MILK_BUCKET,
        &Item::MILK_BUCKET,
        &Item::SUGAR,
        &Item::EGG,
        &Item::SUGAR,
        &Item::WHEAT,
        &Item::WHEAT,
        &Item::WHEAT,
    ]
    .into_iter()
    .enumerate()
    {
        menu.get_behaviour().slots[i + 1].set_stack(ItemStack::new(1, item));
    }
    menu.get_behaviour().slots[0].set_stack(ItemStack::EMPTY.clone());
    menu.on_slot_click(0, 0, SlotActionType::Pickup, &player);
    for i in 1..=3 {
        assert_eq!(
            menu.get_behaviour().slots[i].get_stack().item,
            &Item::BUCKET
        );
    }
    assert_eq!(
        menu.get_behaviour().cursor_stack.lock().unwrap().item,
        &Item::CAKE
    );
}

#[test]
fn remainders_are_given_or_dropped_when_the_grid_stays_occupied() {
    let player = Player::new(3);
    for i in 0..36 {
        player
            .inventory
            .set_stack(i, ItemStack::new(64, &Item::DIRT));
    }
    let grid = Arc::new(CraftingInventory::new(3, 3));
    for (i, item) in [
        &Item::MILK_BUCKET,
        &Item::MILK_BUCKET,
        &Item::MILK_BUCKET,
        &Item::SUGAR,
        &Item::EGG,
        &Item::SUGAR,
        &Item::WHEAT,
        &Item::WHEAT,
        &Item::WHEAT,
    ]
    .into_iter()
    .enumerate()
    {
        grid.set_stack(i, ItemStack::new(2, item));
    }
    let result = crate::crafting::crafting_screen_handler::ResultSlot::new(grid.clone(), None);
    crate::slot::Slot::on_take_item(&result, &player, &ItemStack::new(1, &Item::CAKE));
    let drops = player.drops.lock().unwrap();
    assert_eq!(drops.len(), 3);
    assert!(
        drops
            .iter()
            .all(|s| s.item == &Item::BUCKET && s.item_count == 1)
    );
    assert_eq!(grid.get_stack(0).item_count, 1);
}

#[test]
fn transmute_requires_input_and_preserves_shulker_contents() {
    let grid = CraftingInventory::new(2, 2);
    grid.set_stack(0, ItemStack::new(1, &Item::RED_DYE));
    grid.set_stack(1, ItemStack::new(1, &Item::RED_DYE));
    assert!(match_crafting_recipe(&grid, None).is_none());
    let mut shulker = ItemStack::new(1, &Item::SHULKER_BOX);
    shulker.set_data_component(ContainerImpl {
        items: vec![(7, ItemStack::new(19, &Item::DIAMOND))],
    });
    grid.set_stack(0, shulker);
    let result = match_crafting_recipe(&grid, None).unwrap().stack;
    assert_eq!(result.item, &Item::RED_SHULKER_BOX);
    let contents = result.get_data_component::<ContainerImpl>().unwrap();
    assert_eq!(
        (contents.items[0].0, contents.items[0].1.item_count),
        (7, 19)
    );
    grid.set_stack(2, ItemStack::new(1, &Item::RED_DYE));
    assert!(match_crafting_recipe(&grid, None).is_none());
}

#[test]
fn map_cloning_counts_material_slots_and_copies_map_id() {
    let grid = CraftingInventory::new(2, 2);
    let mut map = ItemStack::new(1, &Item::FILLED_MAP);
    map.set_data_component(MapIdImpl { id: 47 });
    grid.set_stack(0, map);
    for i in 1..=3 {
        grid.set_stack(i, ItemStack::new(9, &Item::MAP));
    }
    let result = match_crafting_recipe(&grid, None).unwrap().stack;
    assert_eq!(result.item_count, 4);
    assert_eq!(result.get_data_component::<MapIdImpl>().unwrap().id, 47);
}

#[test]
fn rockets_keep_flight_duration() {
    let grid = CraftingInventory::new(2, 2);
    grid.set_stack(0, ItemStack::new(1, &Item::PAPER));
    grid.set_stack(1, ItemStack::new(1, &Item::GUNPOWDER));
    grid.set_stack(2, ItemStack::new(1, &Item::GUNPOWDER));
    let explosion = FireworkExplosionImpl::new(
        FireworkExplosionShape::Star,
        vec![0x123456],
        vec![0x654321],
        true,
        false,
    );
    let mut star = ItemStack::new(1, &Item::FIREWORK_STAR);
    star.set_data_component(explosion.clone());
    grid.set_stack(3, star);
    let rocket = match_crafting_recipe(&grid, None).unwrap().stack;
    assert_eq!(
        rocket
            .get_data_component::<FireworksImpl>()
            .unwrap()
            .flight_duration,
        2
    );
    assert_eq!(
        rocket
            .get_data_component::<FireworksImpl>()
            .unwrap()
            .explosions,
        [explosion]
    );
}

#[test]
fn repairs_restore_durability() {
    let grid = CraftingInventory::new(2, 2);
    let mut tool = ItemStack::new(1, &Item::DIAMOND_PICKAXE);
    tool.set_data_component(DamageImpl { damage: 1000 });
    tool.set_data_component(EnchantmentsImpl {
        enchantment: Cow::Owned(vec![
            (&Enchantment::UNBREAKING, 3),
            (&Enchantment::VANISHING_CURSE, 1),
        ]),
    });
    grid.set_stack(0, tool.clone());
    grid.set_stack(1, tool);
    let repaired = match_crafting_recipe(&grid, None).unwrap().stack;
    assert_eq!(repaired.get_damage(), 361);
    assert_eq!(
        repaired
            .get_data_component::<EnchantmentsImpl>()
            .unwrap()
            .enchantment
            .iter()
            .map(|(enchantment, level)| (enchantment.id, *level))
            .collect::<Vec<_>>(),
        [(Enchantment::VANISHING_CURSE.id, 1)]
    );
}

#[test]
fn book_cloning_increments_generation_and_returns_original() {
    let grid = CraftingInventory::new(2, 2);
    let mut book = ItemStack::new(1, &Item::WRITTEN_BOOK);
    book.set_data_component(WrittenBookContentImpl {
        title: "Story".into(),
        author: "Owner".into(),
        pages: vec![TextComponent::text("page")],
        generation: 1,
        resolved: true,
    });
    grid.set_stack(0, book);
    grid.set_stack(1, ItemStack::new(1, &Item::WRITABLE_BOOK));
    let recipe = match_crafting_recipe(&grid, None).unwrap();
    assert_eq!(
        recipe
            .stack
            .get_data_component::<WrittenBookContentImpl>()
            .unwrap()
            .generation,
        2
    );
    assert_eq!(recipe.remaining_items[0].item, &Item::WRITTEN_BOOK);
    grid.set_stack(0, recipe.stack);
    assert!(match_crafting_recipe(&grid, None).is_none());
}

#[test]
fn leather_dyeing_uses_dye_component_color() {
    let grid = CraftingInventory::new(2, 2);
    grid.set_stack(0, ItemStack::new(1, &Item::LEATHER_HELMET));
    grid.set_stack(1, ItemStack::new(1, &Item::RED_DYE));
    assert_eq!(
        match_crafting_recipe(&grid, None)
            .unwrap()
            .stack
            .get_data_component::<DyedColorImpl>()
            .unwrap()
            .rgb as u32,
        DyeColor::Red.texture_diffuse_color() & 0xFFFFFF
    );
}

#[test]
fn banner_cloning_copies_patterns_and_returns_original() {
    let grid = CraftingInventory::new(2, 2);
    let mut banner = ItemStack::new(1, &Item::WHITE_BANNER);
    banner.set_data_component(BannerPatternsImpl {
        layers: vec![BannerPatternLayer {
            pattern: "minecraft:stripe_bottom".into(),
            color: DyeColor::Blue,
        }],
    });
    grid.set_stack(0, banner);
    grid.set_stack(1, ItemStack::new(1, &Item::WHITE_BANNER));
    let recipe = match_crafting_recipe(&grid, None).unwrap();
    assert_eq!(
        recipe
            .stack
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .len(),
        1
    );
    assert_eq!(recipe.remaining_items[0].item_count, 1);
}

#[test]
fn stonecutter_selection_refreshes_output_and_close_returns_input() {
    let player = Player::new(0);
    let mut menu = StonecutterScreenHandler::new(1, &player.inventory);
    menu.input_inventory
        .set_stack(0, ItemStack::new(3, &Item::STONE));
    assert!(menu.on_button_click(&player, 0));
    assert_eq!(*player.properties.lock().unwrap(), [(0, 0)]);
    assert!(!menu.output_inventory.get_stack(0).is_empty());
    menu.on_slot_click(1, 0, SlotActionType::Pickup, &player);
    assert_eq!(menu.input_inventory.get_stack(0).item_count, 2);
    assert!(!menu.output_inventory.get_stack(0).is_empty());
    menu.on_closed(&player);
    assert!(menu.input_inventory.get_stack(0).is_empty());
    assert!(menu.output_inventory.get_stack(0).is_empty());
    assert_eq!(
        (0..36)
            .map(|i| player.inventory.get_stack(i))
            .filter(|s| s.item == &Item::STONE)
            .map(|s| s.item_count)
            .sum::<u8>(),
        2
    );
}

#[test]
fn shulker_slots_reject_direct_and_shift_click_nesting() {
    let player = Player::new(0);
    let input = Arc::new(SimpleInventory::new(27));
    let mut menu = ShulkerBoxScreenHandler::new(1, &player.inventory, input.clone(), &player);
    *menu.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::new(1, &Item::SHULKER_BOX);
    menu.on_slot_click(0, 0, SlotActionType::Pickup, &player);
    assert!(input.is_empty());
    *menu.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::EMPTY.clone();
    player
        .inventory
        .set_stack(9, ItemStack::new(1, &Item::RED_SHULKER_BOX));
    assert!(menu.quick_move(&player, 27).is_empty());
    assert!(input.is_empty());
    assert_eq!(player.inventory.get_stack(9).item_count, 1);
}

#[test]
fn smithing_routes_armor_to_base_and_rejects_wrong_manual_inputs() {
    let player = Player::new(0);
    player
        .inventory
        .set_stack(9, ItemStack::new(1, &Item::DIAMOND_CHESTPLATE));
    let mut menu = SmithingTableScreenHandler::new(1, &player.inventory);
    menu.on_slot_click(4, 0, SlotActionType::QuickMove, &player);
    assert!(menu.input_inventory.get_stack(0).is_empty());
    assert_eq!(
        menu.input_inventory.get_stack(1).item,
        &Item::DIAMOND_CHESTPLATE
    );
    assert!(
        !menu.get_behaviour().slots[0].can_insert(&ItemStack::new(1, &Item::DIAMOND_CHESTPLATE))
    );
}

#[test]
fn cartography_rejects_maps_without_an_id() {
    let player = Player::new(0);
    let mut menu = CartographyTableScreenHandler::new(1, &player.inventory);
    menu.input_inventory
        .set_stack(0, ItemStack::new(1, &Item::FILLED_MAP));
    menu.input_inventory
        .set_stack(1, ItemStack::new(1, &Item::PAPER));
    menu.slots_changed(&player);
    assert!(menu.output_inventory.get_stack(0).is_empty());
}

#[test]
fn exhausted_milk_slots_produce_no_remainders_for_planks() {
    let grid = crate::crafting::crafting_inventory::CraftingInventory::new(3, 3);
    for slot in 0..3 {
        grid.set_stack(slot, ItemStack::new(0, &Item::MILK_BUCKET));
    }
    grid.set_stack(8, ItemStack::new(1, &Item::OAK_LOG));
    let recipe = match_crafting_recipe(&grid, None).unwrap();
    assert_eq!(recipe.stack.item, &Item::OAK_PLANKS);
    assert!(recipe.remaining_items.iter().all(ItemStack::is_empty));
}

#[test]
fn cartography_requires_saved_unlocked_map_below_maximum_scale() {
    let player = Player::new(0);
    let mut menu = CartographyTableScreenHandler::new(1, &player.inventory);
    let mut map = ItemStack::new(1, &Item::FILLED_MAP);
    map.set_data_component(MapIdImpl { id: 47 });
    menu.input_inventory.set_stack(0, map);
    menu.input_inventory
        .set_stack(1, ItemStack::new(1, &Item::PAPER));
    for state in [None, Some((0, true)), Some((4, false))] {
        *player.map_state.lock().unwrap() = state;
        menu.slots_changed(&player);
        assert!(menu.output_inventory.get_stack(0).is_empty());
    }
    *player.map_state.lock().unwrap() = Some((3, false));
    menu.slots_changed(&player);
    assert!(!menu.output_inventory.get_stack(0).is_empty());
}
