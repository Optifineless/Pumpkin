#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures must load and serialize"
)]

use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{BundleContentsImpl, ContainerImpl, CustomDataImpl, FoodImpl},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::{
    Inventory, build_equipment_slots,
    entity_equipment::EntityEquipment,
    inventory::{SimpleInventory, slot_transaction},
    player::{ender_chest_inventory::EnderChestInventory, player_inventory::PlayerInventory},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    bedrock::network_item::NetworkItemDescriptor,
    codec::{
        item_stack_seralizer::{ItemStackSerializer, OptionalItemStackHash},
        var_int::VarInt,
    },
    ser::NetworkWriteExt,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use crate::{
    block::entities::{BlockEntity, barrel::BarrelBlockEntity},
    entity::{Entity, EntityBase, NBTStorage, death_test_world::DeathTestWorld, item::ItemEntity},
};

fn saved_stack(undecodable: bool) -> NbtCompound {
    let mut components = NbtCompound::new();
    if undecodable {
        components.put_string("minecraft:food", "bad".into());
    } else {
        let mut value = NbtCompound::new();
        value.put_int("a", 1);
        components.put_compound("minecraft:future_thing", value);
    }
    let mut stack = NbtCompound::new();
    stack.put_string("id", "minecraft:stone".into());
    stack.put_int("count", 7);
    stack.put_compound("components", components);
    stack
}

fn tag_bytes(tag: &NbtTag) -> Vec<u8> {
    let mut bytes = Vec::new();
    tag.clone()
        .serialize(&mut pumpkin_nbt::serializer::NbtWriteHelperJava::new(
            &mut bytes,
        ))
        .unwrap();
    bytes
}

fn assert_saved(expected: &NbtCompound, actual: &NbtCompound) {
    assert_eq!(actual.get_string("id"), expected.get_string("id"));
    assert_eq!(actual.get_int("count"), expected.get_int("count"));
    let expected = expected.get_compound("components").unwrap();
    let actual = actual.get_compound("components").unwrap();
    for (name, tag) in &expected.child_tags {
        assert_eq!(
            tag_bytes(actual.get(name).unwrap()),
            tag_bytes(tag),
            "{name}"
        );
    }
}

fn inventory_roundtrip(undecodable: bool, ender: bool) {
    let expected = saved_stack(undecodable);
    let mut entry = expected.clone();
    entry.put_byte("Slot", 0);
    let key = if ender { "EnderItems" } else { "Inventory" };
    let mut saved = NbtCompound::new();
    saved.put_list(key, vec![NbtTag::Compound(entry)]);
    for _ in 0..2 {
        let inventory: Box<dyn NBTStorage> = if ender {
            Box::new(EnderChestInventory::new())
        } else {
            Box::new(PlayerInventory::new(
                Arc::new(Mutex::new(EntityEquipment::new())),
                Arc::new(build_equipment_slots()),
            ))
        };
        inventory.read_nbt_non_mut(&saved);
        saved = NbtCompound::new();
        inventory.write_nbt(&mut saved);
        let entries = saved.get_list(key).unwrap();
        assert_eq!(entries.len(), 1, "saved slot disappeared");
        assert_saved(&expected, entries[0].extract_compound().unwrap());
    }
}

fn barrel_roundtrip(undecodable: bool) {
    let expected = saved_stack(undecodable);
    let mut entry = expected.clone();
    entry.put_byte("Slot", 0);
    let mut saved = NbtCompound::new();
    saved.put_list("Items", vec![NbtTag::Compound(entry)]);
    for _ in 0..2 {
        let barrel = BarrelBlockEntity::from_nbt(&saved, BlockPos::new(0, 64, 0));
        saved = NbtCompound::new();
        barrel.write_nbt(&mut saved);
        let entries = saved.get_list("Items").unwrap();
        assert_eq!(entries.len(), 1, "barrel slot disappeared");
        assert_saved(&expected, entries[0].extract_compound().unwrap());
    }
}

fn nested_roundtrip(undecodable: bool) {
    let inner = saved_stack(undecodable);
    let mut entry = NbtCompound::new();
    entry.put_int("slot", 0);
    entry.put_compound("item", inner.clone());
    let mut components = NbtCompound::new();
    components.put_list("minecraft:container", vec![NbtTag::Compound(entry)]);
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:shulker_box".into());
    saved.put_int("count", 1);
    saved.put_compound("components", components);
    for _ in 0..2 {
        let stack = ItemStack::read_item_stack(&saved).unwrap();
        assert_eq!(
            stack
                .get_data_component::<ContainerImpl>()
                .unwrap()
                .items
                .len(),
            1
        );
        saved = NbtCompound::new();
        stack.write_item_stack(&mut saved);
        let entry = &saved
            .get_compound("components")
            .unwrap()
            .get_list("minecraft:container")
            .unwrap()[0];
        assert_saved(
            &inner,
            entry
                .extract_compound()
                .unwrap()
                .get_compound("item")
                .unwrap(),
        );
    }
    // Recurse once more through a bundle around the shulker.
    let mut bundle = ItemStack::new(1, &Item::BUNDLE);
    bundle.set_data_component(BundleContentsImpl {
        items: vec![ItemStack::read_item_stack(&saved).unwrap()],
        selected_item: -1,
    });
    bundle.write_item_stack(&mut saved);
    let restored = ItemStack::read_item_stack(&saved).unwrap();
    let shulker = &restored
        .get_data_component::<BundleContentsImpl>()
        .unwrap()
        .items[0];
    let contents = shulker.get_data_component::<ContainerImpl>().unwrap();
    let mut written = NbtCompound::new();
    contents.items[0].1.write_item_stack(&mut written);
    assert_saved(&inner, &written);
}

async fn dropped_roundtrip(undecodable: bool) {
    let fixture = DeathTestWorld::new().await;
    let expected = saved_stack(undecodable);
    let mut saved = NbtCompound::new();
    saved.put_compound("Item", expected.clone());
    for _ in 0..2 {
        let entity = ItemEntity::new_empty(Entity::new(
            fixture.world(),
            Vector3::new(0.0, 64.0, 0.0),
            &EntityType::ITEM,
        ));
        entity.read_custom_nbt(&saved);
        saved = NbtCompound::new();
        entity.write_custom_nbt(&mut saved);
        assert_saved(&expected, saved.get_compound("Item").unwrap());
    }
}

#[test]
fn raw_components_player_unknown() {
    inventory_roundtrip(false, false);
}
#[test]
fn raw_components_player_undecodable() {
    inventory_roundtrip(true, false);
}
#[test]
fn raw_components_ender_unknown() {
    inventory_roundtrip(false, true);
}
#[test]
fn raw_components_ender_undecodable() {
    inventory_roundtrip(true, true);
}
#[test]
fn raw_components_barrel_unknown() {
    barrel_roundtrip(false);
}
#[test]
fn raw_components_barrel_undecodable() {
    barrel_roundtrip(true);
}
#[test]
fn raw_components_nested_unknown() {
    nested_roundtrip(false);
}
#[test]
fn raw_components_nested_undecodable() {
    nested_roundtrip(true);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_components_dropped_unknown() {
    dropped_roundtrip(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_components_dropped_undecodable() {
    dropped_roundtrip(true).await;
}

#[test]
fn raw_components_removal_and_decoded_control() {
    let mut saved = saved_stack(false);
    let mut components = saved.get_compound("components").unwrap().clone();
    components.put_compound("!minecraft:food", NbtCompound::new());
    components.put_int("minecraft:damage", 3);
    saved.put_compound("components", components);
    let stack = ItemStack::read_item_stack(&saved).unwrap();
    assert!(!stack.has_data_component(DataComponent::Food));
    assert!(stack.has_data_component(DataComponent::Damage));
    let mut written = NbtCompound::new();
    stack.write_item_stack(&mut written);
    assert_saved(&saved, &written);

    let mut food = NbtCompound::new();
    food.put_int("nutrition", 4);
    food.put_float("saturation", 0.5);
    let mut components = NbtCompound::new();
    components.put_compound("minecraft:food", food);
    saved.put_compound("components", components);
    let stack = ItemStack::read_item_stack(&saved).unwrap();
    assert_eq!(stack.get_data_component::<FoodImpl>().unwrap().nutrition, 4);
    stack.write_item_stack(&mut written);
    let restored = ItemStack::read_item_stack(&written).unwrap();
    assert!(stack.are_equal(&restored));
}

#[test]
fn raw_components_network_omits_without_mutation() {
    for undecodable in [false, true] {
        let saved = saved_stack(undecodable);
        let stack = ItemStack::read_item_stack(&saved).unwrap();
        let plain = ItemStack::new(7, &Item::STONE);
        let encode = |stack: &ItemStack| {
            let serializer = ItemStackSerializer(Cow::Borrowed(stack));
            let mut bytes = Vec::new();
            serializer
                .write_with_version(&mut bytes, &CURRENT_MC_VERSION)
                .unwrap();
            serializer
                .write_length_prefixed_with_version(&mut bytes, &CURRENT_MC_VERSION)
                .unwrap();
            serializer
                .write_item_cost_with_version(&mut bytes, &CURRENT_MC_VERSION)
                .unwrap();
            serializer
                .write_template_with_version(&mut bytes, &CURRENT_MC_VERSION)
                .unwrap();
            bytes
        };
        assert_eq!(encode(&stack), encode(&plain));
        let descriptor = NetworkItemDescriptor::from(&stack);
        assert_eq!(descriptor.stack_size, 7);
        assert!(descriptor.nbt_data.root_tag.is_empty());
        let mut written = NbtCompound::new();
        stack.write_item_stack(&mut written);
        assert_saved(&saved, &written);
    }
}

#[test]
fn raw_components_deposit_preserves_different_and_merges_equal() {
    let first = saved_stack(false);
    let mut different = first.clone();
    let mut components = NbtCompound::new();
    let mut value = NbtCompound::new();
    value.put_int("a", 2);
    components.put_compound("minecraft:future_thing", value);
    different.put_compound("components", components);
    let inventory = SimpleInventory::new(1);
    inventory.set_stack(0, ItemStack::read_item_stack(&first).unwrap());
    let mut incoming = ItemStack::read_item_stack(&different).unwrap();
    slot_transaction::deposit(&inventory, 0, &mut incoming, 64);
    assert_eq!(incoming.item_count, 7);
    assert_eq!(inventory.get_stack(0).item_count, 7);
    let mut written = NbtCompound::new();
    incoming.write_item_stack(&mut written);
    assert_saved(&different, &written);
    inventory.get_stack(0).write_item_stack(&mut written);
    assert_saved(&first, &written);

    let mut incoming = ItemStack::read_item_stack(&first).unwrap();
    slot_transaction::deposit(&inventory, 0, &mut incoming, 64);
    assert!(incoming.is_empty());
    assert_eq!(inventory.get_stack(0).item_count, 14);
}

#[test]
fn raw_components_unknown_removal_and_noncanonical_known_removal() {
    let mut saved = saved_stack(false);
    let mut components = saved.get_compound("components").unwrap().clone();
    components.put_compound("!minecraft:future_removed", NbtCompound::new());
    let mut expected = saved.clone();
    components.put_compound("!minecraft:food", NbtCompound::new());
    expected.put_compound("components", components.clone());
    components.put_string("!minecraft:food", "original marker payload".into());
    saved.put_compound("components", components);
    for _ in 0..2 {
        let stack = ItemStack::read_item_stack(&saved).unwrap();
        assert!(!stack.has_data_component(DataComponent::Food));
        stack.write_item_stack(&mut saved);
        assert_saved(&expected, &saved);
    }
}

#[test]
fn raw_components_exact_float_bits_and_nested_equality() {
    let make = |value: f64| {
        let mut saved = saved_stack(false);
        let mut components = NbtCompound::new();
        let mut nested = NbtCompound::new();
        nested.put_list("values", vec![NbtTag::Double(value)]);
        components.put_compound("minecraft:future_thing", nested);
        saved.put_compound("components", components);
        ItemStack::read_item_stack(&saved).unwrap()
    };
    let positive = make(0.0);
    let negative = make(-0.0);
    assert!(!positive.are_items_and_components_equal(&negative));
    let nan = make(f64::from_bits(0x7ff8_0000_0000_0042));
    assert!(nan.are_items_and_components_equal(&nan.clone()));
    let wrap = |stack| {
        let mut bundle = ItemStack::new(1, &Item::BUNDLE);
        bundle.set_data_component(BundleContentsImpl {
            items: vec![stack],
            selected_item: -1,
        });
        bundle
    };
    assert!(!wrap(positive).are_items_and_components_equal(&wrap(negative)));
}

#[test]
fn raw_components_copies_recipes_and_nested_templates() {
    use pumpkin_data::{
        data_component_impl::{PotDecorationsImpl, UseRemainderImpl},
        recipes::RecipeResultStruct,
    };
    for undecodable in [false, true] {
        let saved = saved_stack(undecodable);
        let mut stack = ItemStack::read_item_stack(&saved).unwrap();
        let split = stack.split_off(2);
        assert_eq!(stack.item_count + split.item_count, 7);
        let result = RecipeResultStruct {
            id: "minecraft:diamond",
            count: 7,
            components: None,
        }
        .assemble(Some(&stack), 0);
        let mut written = NbtCompound::new();
        result.write_item_stack(&mut written);
        assert_eq!(
            written.get_compound("components"),
            saved.get_compound("components")
        );

        // Both codecs contain an item template rather than an ordinary container slot.
        let mut components = NbtCompound::new();
        components.put_compound("minecraft:use_remainder", saved.clone());
        let mut faces = NbtCompound::new();
        faces.put_compound("back", saved.clone());
        components.put_compound("minecraft:pot_decorations", faces);
        let mut outer = saved.clone();
        outer.put_compound("components", components);
        let outer = ItemStack::read_item_stack(&outer).unwrap();
        let remainder = outer
            .get_data_component::<UseRemainderImpl>()
            .unwrap()
            .create()
            .unwrap();
        remainder.write_item_stack(&mut written);
        assert_saved(&saved, &written);
        let pot = outer.get_data_component::<PotDecorationsImpl>().unwrap();
        pot.decorations[0]
            .as_ref()
            .unwrap()
            .to_item_stack()
            .write_item_stack(&mut written);
        assert_saved(&saved, &written);
    }
}

#[test]
fn raw_components_explicit_edits_override_only_the_named_component() {
    let mut saved = saved_stack(true);
    // The persistent name lookup also accepts unqualified component aliases.
    let mut components = saved.get_compound("components").unwrap().clone();
    components.put_string("food", "also bad".into());
    saved.put_compound("components", components);
    let mut stack = ItemStack::read_item_stack(&saved).unwrap();
    // An unsuccessful mutable lookup must leave the undecodable value intact.
    assert!(stack.get_data_component_mut::<FoodImpl>().is_none());
    let mut written = NbtCompound::new();
    stack.write_item_stack(&mut written);
    assert_saved(&saved, &written);
    stack.set_data_component(FoodImpl {
        nutrition: 4,
        saturation: 0.5,
        can_always_eat: false,
    });
    stack.write_item_stack(&mut written);
    let restored = ItemStack::read_item_stack(&written).unwrap();
    assert_eq!(
        restored.get_data_component::<FoodImpl>().unwrap().nutrition,
        4
    );
    stack.remove_data_component(DataComponent::Food);
    stack.write_item_stack(&mut written);
    assert!(
        written
            .get_compound("components")
            .unwrap()
            .has("!minecraft:food")
    );
    assert!(
        !written
            .get_compound("components")
            .unwrap()
            .has("minecraft:food")
    );
}

#[test]
fn raw_components_network_nested_items_and_generic_inventory_nbt() {
    use pumpkin_data::data_component_impl::{BlockEntityDataImpl, DataComponentImpl};
    for undecodable in [false, true] {
        let saved = saved_stack(undecodable);
        let stack = ItemStack::read_item_stack(&saved).unwrap();
        let encode = |stack: &ItemStack| {
            let mut bytes = Vec::new();
            ItemStackSerializer(Cow::Borrowed(stack))
                .write(&mut bytes)
                .unwrap();
            bytes
        };
        let wrap = |stack| {
            let mut bundle = ItemStack::new(1, &Item::BUNDLE);
            bundle.set_data_component(BundleContentsImpl {
                items: vec![stack],
                selected_item: -1,
            });
            bundle
        };
        assert_eq!(
            encode(&wrap(stack.clone())),
            encode(&wrap(ItemStack::new(7, &Item::STONE)))
        );
        let mut payload = NbtCompound::new();
        payload.put_string("id", "minecraft:chest".into());
        payload.put_list("Items", vec![NbtTag::Compound(saved.clone())]);
        let component = BlockEntityDataImpl { nbt: payload };
        let mut chest = ItemStack::new(1, &Item::CHEST);
        chest.set_data_component(component.clone());
        assert_eq!(encode(&chest), encode(&ItemStack::new(1, &Item::CHEST)));
        // A client hashes exactly the visible component patch, not withheld parents.
        let mut claim = Vec::new();
        claim.put_bool(true).unwrap();
        claim.put_var_int(&VarInt::from(Item::CHEST.id)).unwrap();
        claim.put_var_int(&VarInt(1)).unwrap();
        claim.put_var_int(&VarInt(0)).unwrap();
        claim.put_var_int(&VarInt(0)).unwrap();
        assert!(
            OptionalItemStackHash::read(&mut claim.as_slice())
                .unwrap()
                .hash_equals(&chest)
        );
        assert_eq!(
            chest
                .get_data_component::<BlockEntityDataImpl>()
                .unwrap()
                .write_data(),
            component.write_data()
        );
        // A fully decodable inventory keeps its normal network representation.
        let mut plain = NbtCompound::new();
        ItemStack::new(7, &Item::STONE).write_item_stack(&mut plain);
        let mut payload = NbtCompound::new();
        payload.put_list("Items", vec![NbtTag::Compound(plain)]);
        chest.set_data_component(BlockEntityDataImpl { nbt: payload });
        assert_ne!(encode(&chest), encode(&ItemStack::new(1, &Item::CHEST)));
    }
}

#[test]
fn raw_components_known_decode_and_invalid_identity_controls() {
    use pumpkin_data::data_component_impl::DamageImpl;
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:iron_sword".into());
    saved.put_int("count", 1);
    let mut components = NbtCompound::new();
    components.put_int("minecraft:damage", 12);
    saved.put_compound("components", components);
    let stack = ItemStack::read_item_stack(&saved).unwrap();
    assert_eq!(stack.get_data_component::<DamageImpl>().unwrap().damage, 12);
    let mut written = NbtCompound::new();
    stack.write_item_stack(&mut written);
    assert_saved(&saved, &written);
    for count in [NbtTag::String("7".into()), NbtTag::Int(0), NbtTag::Int(100)] {
        let mut invalid = saved.clone();
        invalid.put("count", count);
        assert!(ItemStack::read_item_stack(&invalid).is_none());
    }
    for id in [
        NbtTag::Int(1),
        NbtTag::String("minecraft:future_item".into()),
    ] {
        let mut invalid = saved.clone();
        invalid.put("id", id);
        assert!(ItemStack::read_item_stack(&invalid).is_none());
    }
}

#[test]
fn raw_components_convenience_setters_preserve_unrelated_raw_data() {
    let mut saved = saved_stack(false);
    let mut components = saved.get_compound("components").unwrap().clone();
    components.put_int("custom_name", 123);
    components.put_string("repair_cost", "bad".into());
    components.put_string("custom_data", "bad".into());
    saved.put_compound("components", components);
    let mut stack = ItemStack::read_item_stack(&saved).unwrap();
    stack.set_custom_name("renamed".into());
    stack.set_repair_cost(0);
    stack.set_custom_data("plugin", "marker", NbtTag::Int(42));
    let mut written = NbtCompound::new();
    stack.write_item_stack(&mut written);
    let restored = ItemStack::read_item_stack(&written).unwrap();
    assert_eq!(restored.get_hover_name(), "renamed");
    assert_eq!(
        restored.get_custom_data("plugin", "marker"),
        Some(NbtTag::Int(42))
    );
    let components = written.get_compound("components").unwrap();
    assert!(!components.has("repair_cost"));
    assert!(!components.has("custom_name"));
    assert!(!components.has("custom_data"));
    let original = saved
        .get_compound("components")
        .unwrap()
        .get("minecraft:future_thing")
        .unwrap();
    assert_eq!(
        tag_bytes(components.get("minecraft:future_thing").unwrap()),
        tag_bytes(original)
    );

    // Keep the existing cleanup of decoded empty namespaces on missing-key removal.
    let mut custom_data = NbtCompound::new();
    custom_data.put_compound("empty", NbtCompound::new());
    stack.set_data_component(CustomDataImpl { data: custom_data });
    stack.remove_custom_data("empty", "missing");
    assert!(stack.get_data_component::<CustomDataImpl>().is_none());
    stack.write_item_stack(&mut written);
    assert_eq!(
        tag_bytes(
            written
                .get_compound("components")
                .unwrap()
                .get("minecraft:future_thing")
                .unwrap()
        ),
        tag_bytes(original)
    );
}
