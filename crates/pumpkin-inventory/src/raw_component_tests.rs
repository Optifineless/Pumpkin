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
