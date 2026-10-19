#![expect(clippy::unwrap_used, reason = "Known vanilla codec and hash fixtures")]

use pumpkin_data::{
    data_component::DataComponent, data_component_impl::PotDecorationsImpl, item::Item,
    item_stack::ItemStack,
};

use crate::{
    VarInt,
    codec::{data_component::DataComponentCodec, item_stack_seralizer::OptionalItemStackHash},
    ser::NetworkWriteExt,
};

fn pot_template_wire() -> Vec<u8> {
    // PotDecorations.STREAM_CODEC: back brick count=2 with damage=3 and removed
    // unbreakable; left, right and front absent. Template patch counts precede values.
    let mut wire = vec![1];
    wire.write_var_int(&VarInt(i32::from(Item::BRICK.id)))
        .unwrap();
    wire.extend_from_slice(&[2, 1, 1]);
    wire.write_var_int(&VarInt(i32::from(DataComponent::Damage.to_id())))
        .unwrap();
    wire.push(3);
    wire.write_var_int(&VarInt(i32::from(DataComponent::Unbreakable.to_id())))
        .unwrap();
    wire.extend_from_slice(&[0, 0, 0]);
    wire
}

#[test]
fn b1020b_g09_pot_template_wire_keeps_additions_and_removals() {
    let wire = pot_template_wire();
    let decorations = PotDecorationsImpl::deserialize(&mut wire.as_slice()).unwrap();
    let mut encoded = Vec::new();
    decorations.serialize(&mut encoded).unwrap();
    assert_eq!(encoded, wire);
}

#[test]
fn b1020b_g09_pot_template_independent_hash_claims_agree() {
    let decorations = PotDecorationsImpl::deserialize(&mut pot_template_wire().as_slice()).unwrap();
    let mut stack = ItemStack::new(1, &Item::DECORATED_POT);
    stack.set_data_component(decorations);

    // HashOps over {back:{id:"minecraft:brick",count:2,components:
    // {"minecraft:damage":3,"!minecraft:unbreakable":{}}}}. These fixed bytes
    // come from an independent CRC32C encoder, not the component's get_hash.
    for (checksum, matches) in [(0x0a92_6050u32, true), (0, false)] {
        let mut claim = vec![1];
        claim
            .write_var_int(&VarInt(i32::from(Item::DECORATED_POT.id)))
            .unwrap();
        claim.extend_from_slice(&[1, 1]);
        claim
            .write_var_int(&VarInt(i32::from(DataComponent::PotDecorations.to_id())))
            .unwrap();
        claim.extend_from_slice(&checksum.to_be_bytes());
        claim.push(0);
        let claim = OptionalItemStackHash::read(&mut claim.as_slice()).unwrap();
        assert_eq!(claim.hash_equals(&stack), matches);
    }
}
