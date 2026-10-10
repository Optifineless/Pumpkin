use super::*;
use crate::{
    server::combat_test_support,
    world::loot::{self, LootContextParameters},
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{BeesImpl, BundleContentsImpl},
    item::Item,
    item_stack::ItemStack,
};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silk_touch_hive_drop_bundle_persistence_placement_keeps_occupants() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 0);
    let hive = Arc::new(BeehiveBlockEntity::new(pos));
    // BeehiveBlockEntity.Occupant.CODEC fixture, independently specified.
    let mut entity = NbtCompound::new();
    entity.put_string("id", "minecraft:bee".to_owned());
    entity.put_int("Age", -1200);
    entity.put_bool("HasNectar", true);
    let mut occupant = NbtCompound::new();
    occupant.put_compound("entity_data", entity);
    occupant.put_int("ticks_in_hive", 37);
    occupant.put_int("min_ticks_in_hive", 2400);
    *hive.bees.lock().unwrap() = Some(vec![NbtTag::Compound(occupant)]);
    world.add_block_entity(hive.clone());
    let mut pickaxe = ItemStack::new(1, &Item::DIAMOND_PICKAXE);
    pickaxe.add_enchantment(&pumpkin_data::Enchantment::SILK_TOUCH, 1);
    let mut params = loot::build_block_loot_context(
        &world,
        &pos,
        &LootContextParameters {
            tool: Some(pickaxe),
            ..Default::default()
        },
    );
    // This fixture skips datapack boot; resolve the Silk Touch predicate from generated vanilla data.
    params.registry = None;
    assert_eq!(
        params.block_entity_components[&DataComponent::Bees]
            .write_data()
            .extract_list()
            .unwrap()
            .len(),
        1
    );
    let table = loot::get_loot_table("minecraft:blocks/beehive").unwrap();
    let mut drop = table.generate_loot_with_context(17, &params).remove(0);
    let bees = drop.get_data_component::<BeesImpl>().unwrap().clone();
    assert_eq!(bees.bees.len(), 1);
    assert_eq!(
        (bees.bees[0].ticks_in_hive, bees.bees[0].min_ticks_in_hive),
        (37, 2400)
    );
    let mut contents = BundleContentsImpl {
        items: Vec::new(),
        selected_item: -1,
    };
    assert!(contents.try_insert(&mut drop));
    assert_eq!(contents.get_weight(), 64);
    let mut bundle = ItemStack::new(1, &Item::BUNDLE);
    bundle.set_data_component(contents);
    let mut nbt = NbtCompound::new();
    bundle.write_item_stack(&mut nbt);
    let bundle = ItemStack::read_item_stack(&nbt).unwrap();
    let restored = &bundle
        .get_data_component::<BundleContentsImpl>()
        .unwrap()
        .items[0];
    let placed = BeehiveBlockEntity::new(pos);
    placed.apply_item_components(restored);
    let mut placed_stack = ItemStack::new(1, &Item::BEEHIVE);
    placed.collect_item_components(&mut placed_stack);
    assert_eq!(placed_stack.get_data_component::<BeesImpl>(), Some(&bees));
    placed.apply_item_components(&ItemStack::new(1, &Item::BEEHIVE));
    assert!(placed.bees.lock().unwrap().as_ref().unwrap().is_empty());
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}
