use pumpkin_data::{
    data_component_impl::{BlockEntityDataImpl, ChargedProjectilesImpl, PotDecorationsImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use crate::{Inventory, SimpleInventory, inventory::slot_transaction};

#[derive(Clone, Copy)]
enum Payload {
    BlockEntity,
    Projectiles,
    PotFace,
}

fn chest(value: f64, opaque: bool, kind: Payload) -> ItemStack {
    let mut payload = NbtCompound::new();
    if opaque {
        let mut components = NbtCompound::new();
        components.put_double("minecraft:future_thing", value);
        let mut inner = NbtCompound::new();
        inner.put_string("id", "minecraft:stone".into());
        inner.put_int("count", 7);
        inner.put_compound("components", components);
        payload.put_list("Items", vec![NbtTag::Compound(inner)]);
    } else {
        payload.put_double("ordinary", value);
    }
    let mut stack = ItemStack::new(1, &Item::CHEST);
    stack.set_data_component(BlockEntityDataImpl { nbt: payload });
    let mut saved = NbtCompound::new();
    stack.write_item_stack(&mut saved);
    match kind {
        Payload::BlockEntity => ItemStack::read_item_stack(&saved).unwrap(),
        Payload::Projectiles => {
            let mut projectile = NbtCompound::new();
            projectile.put_string("id", "minecraft:stone".into());
            projectile.put_int("count", 7);
            if opaque {
                let mut components = NbtCompound::new();
                components.put_double("minecraft:future_thing", value);
                projectile.put_compound("components", components);
            } else {
                projectile.put_double("ordinary", value);
            }
            let mut stack = ItemStack::new(1, &Item::CHEST);
            stack.set_data_component(ChargedProjectilesImpl {
                projectiles: vec![projectile],
            });
            stack
        }
        Payload::PotFace => {
            let mut faces = NbtCompound::new();
            faces.put_compound("back", saved);
            let mut stack = ItemStack::new(1, &Item::CHEST);
            stack.set_data_component(
                PotDecorationsImpl::read_data(&NbtTag::Compound(faces)).unwrap(),
            );
            stack
        }
    }
}

fn check_deposit(kind: Payload) {
    let original = chest(0.0, true, kind);
    let mut incoming = chest(-0.0, true, kind);
    let expected = incoming.clone();
    let inventory = SimpleInventory::new(1);
    inventory.set_stack(0, original.clone());
    slot_transaction::deposit(&inventory, 0, &mut incoming, 64);
    assert_eq!(inventory.get_stack(0).item_count, 1);
    assert_eq!(incoming.item_count, 1);
    assert!(incoming.are_equal(&expected));
    assert!(inventory.get_stack(0).are_equal(&original));
    // Decodable NBT retains its established numerical equality.
    assert!(chest(0.0, false, kind).are_equal(&chest(-0.0, false, kind)));
}

#[test]
fn raw_components_generic_nested_items_do_not_merge_different_float_bits() {
    check_deposit(Payload::BlockEntity);
}
#[test]
fn raw_components_projectile_items_do_not_merge_different_float_bits() {
    check_deposit(Payload::Projectiles);
}
#[test]
fn raw_components_pot_face_items_do_not_merge_different_float_bits() {
    check_deposit(Payload::PotFace);
}

#[test]
fn raw_components_grindstone_result_clears_both_enchantment_payloads() {
    use crate::{ext_review_tests::Player, grindstone_screen_handler::GrindstoneScreenHandler};
    use std::sync::Arc;

    let mut components = NbtCompound::new();
    components.put_string("enchantments", "future enchantments".into());
    components.put_string("stored_enchantments", "future stored enchantments".into());
    components.put_int("example:unrelated", 37);
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:diamond_sword".into());
    saved.put_int("count", 1);
    saved.put_compound("components", components);
    let input = ItemStack::read_item_stack(&saved).unwrap();
    assert_eq!(input.raw_components.len(), 3);
    let inventory = Arc::new(SimpleInventory::new(3));
    inventory.set_stack(GrindstoneScreenHandler::INPUT_SLOT, input);
    inventory.set_stack(
        GrindstoneScreenHandler::ADDITIONAL_SLOT,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
    );
    let player = Player::new(1);
    let mut handler = GrindstoneScreenHandler::new(1, &player.inventory, inventory.clone());
    handler.create_result();
    let result = inventory.get_stack(GrindstoneScreenHandler::RESULT_SLOT);
    assert!(!result.is_empty());
    assert_eq!(result.raw_components.len(), 1);
    assert_eq!(&*result.raw_components[0].0, "example:unrelated");
    result.write_item_stack(&mut saved);
    let restored = ItemStack::read_item_stack(&saved).unwrap();
    assert!(restored.are_equal(&result));
    assert!(!restored.has_enchantments());
}
