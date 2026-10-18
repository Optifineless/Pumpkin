use pumpkin_data::{data_component::DataComponent, entity::EntityType, item_stack::ItemStack};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    VarInt,
    codec::{data_component, item_stack_seralizer::ItemStackSerializer},
    ser::NetworkWriteExt,
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::io::Cursor;

#[test]
fn mob_visibility_wire_fixture_preserves_full_item_patch() -> Result<(), Box<dyn std::error::Error>>
{
    // MobVisibility.STREAM_CODEC: tag holder-set + namespaced key + big-endian float.
    let payload = b"\x00\x13minecraft:skeletons\x3e\x80\x00\x00";
    let mut input = payload.as_slice();
    let value = data_component::deserialize(DataComponent::MobVisibility, &mut input)?;
    assert!(input.is_empty());
    let nbt = value.write_data();
    let component = nbt.extract_compound().ok_or("component NBT")?;
    assert_eq!(component.get_float("visibility"), Some(0.25));
    assert_eq!(
        component.get_string("targeting_entity_types"),
        Some("#minecraft:skeletons")
    );
    let mut written = Vec::new();
    data_component::serialize(DataComponent::MobVisibility, value.as_ref(), &mut written)?;
    assert_eq!(written, payload);

    for shorthand in [
        b"\x00\x09skeletons\x3e\x80\x00\x00".as_slice(),
        b"\x00\x0a:skeletons\x3e\x80\x00\x00".as_slice(),
    ] {
        let mut input = shorthand;
        let component = data_component::deserialize(DataComponent::MobVisibility, &mut input)?;
        written.clear();
        data_component::serialize(
            DataComponent::MobVisibility,
            component.as_ref(),
            &mut written,
        )?;
        assert_eq!(written, payload);
    }

    // ItemStack.OPTIONAL_STREAM_CODEC: stone, one component, no removals.
    let mut fixture = vec![1, 1, 1, 0, DataComponent::MobVisibility.to_id()];
    fixture.extend(payload);
    let mut cursor = Cursor::new(&fixture);
    let item = ItemStackSerializer::read(&mut cursor)?;
    assert_eq!(cursor.position(), fixture.len() as u64);
    assert!(item.0.has_data_component(DataComponent::MobVisibility));
    let mut saved = NbtCompound::new();
    item.0.write_item_stack(&mut saved);
    let stack = ItemStack::read_item_stack(&saved).ok_or("persist network component")?;
    written.clear();
    ItemStackSerializer::from(stack)
        .write_with_version(&mut written, &JavaMinecraftVersion::V_26_3)?;
    assert_eq!(written, fixture);
    Ok(())
}

#[test]
fn mob_visibility_wire_keeps_nan_and_rejects_invalid_holder_sets()
-> Result<(), Box<dyn std::error::Error>> {
    // Direct HolderSet with one skeleton. ByteBufCodecs.FLOAT does not enforce CODEC bounds.
    let mut payload = vec![2];
    payload.write_var_int(&VarInt(i32::from(EntityType::SKELETON.id)))?;
    payload.extend(f32::NAN.to_be_bytes());
    let mut input = payload.as_slice();
    let component = data_component::deserialize(DataComponent::MobVisibility, &mut input)?;
    assert!(
        component
            .write_data()
            .extract_compound()
            .and_then(|tag| tag.get_float("visibility"))
            .is_some_and(f32::is_nan)
    );
    assert!(input.is_empty());
    let stack = ItemStack::new_with_component(
        1,
        &pumpkin_data::item::Item::STONE,
        vec![(DataComponent::MobVisibility, Some(component))],
    );
    assert!(stack.are_items_and_components_equal(&stack.clone()));
    let mut written = Vec::new();
    data_component::serialize(
        DataComponent::MobVisibility,
        stack.patch[0].1.as_ref().ok_or("component patch")?.as_ref(),
        &mut written,
    )?;
    assert_eq!(written, payload);

    let float_start = payload.len() - 4;
    let mut decoded = Vec::new();
    for value in [0.0f32, -0.0, f32::from_bits(0x7fc00001), f32::NAN] {
        payload[float_start..].copy_from_slice(&value.to_be_bytes());
        decoded.push(data_component::deserialize(
            DataComponent::MobVisibility,
            &mut payload.as_slice(),
        )?);
    }
    assert!(!decoded[0].equal(decoded[1].as_ref()));
    assert!(decoded[2].equal(decoded[3].as_ref()));

    for payload in [
        &[0xff, 0xff, 0xff, 0xff, 0x0f][..],    // Negative holder count.
        &[0x82, 0x20][..],                      // 4,097 holders exceeds the shared holder bound.
        &[2, 0xff, 0xff, 0x03, 0, 0, 0, 0][..], // Unknown registry id.
        &[1, 0, 0][..],                         // Truncated float.
        &[0, 1, b'!'][..],                      // Unknown/invalid tag identifier.
        b"\x00\x0a#skeletons\x3e\x80\x00\x00".as_slice(), // Identifier cannot include '#'.
    ] {
        let mut input = payload;
        assert!(data_component::deserialize(DataComponent::MobVisibility, &mut input).is_err());
    }
    Ok(())
}
