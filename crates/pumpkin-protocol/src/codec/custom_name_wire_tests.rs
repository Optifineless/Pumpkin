use super::*;
use crate::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_data::item::Item;
use pumpkin_nbt::{compound::NbtCompound, serializer::NbtWriteHelperJava, tag::NbtTag};
use std::io::Cursor;

#[test]
fn vanilla_custom_name_wire_fixtures_preserve_full_item_stack_components() {
    for payload in [
        include_bytes!("../../../pumpkin-data/tests/fixtures/custom_names/styled.bin").as_slice(),
        include_bytes!("../../../pumpkin-data/tests/fixtures/custom_names/translation.bin")
            .as_slice(),
        include_bytes!("../../../pumpkin-data/tests/fixtures/custom_names/score.bin").as_slice(),
        include_bytes!("../../../pumpkin-data/tests/fixtures/custom_names/nbt.bin").as_slice(),
        include_bytes!("../../../pumpkin-data/tests/fixtures/custom_names/object.bin").as_slice(),
        include_bytes!("../../../pumpkin-data/tests/fixtures/custom_names/future.bin").as_slice(),
    ] {
        // ItemStack.OPTIONAL_STREAM_CODEC header, followed by independent ByteBufCodecs.TAG bytes.
        let mut fixture = vec![1];
        fixture
            .write_var_int(&VarInt(i32::from(Item::TRIPWIRE_HOOK.id)))
            .unwrap();
        fixture.extend([1, 0, DataComponent::CustomName.to_id()]);
        fixture.extend(payload);
        let mut cursor = Cursor::new(&fixture);
        let item = ItemStackSerializer::read(&mut cursor).unwrap();
        assert_eq!(cursor.position(), fixture.len() as u64);
        let mut written = Vec::new();
        item.write_with_version(&mut written, &JavaMinecraftVersion::V_26_3)
            .unwrap();
        let restored = ItemStackSerializer::read(&mut Cursor::new(written)).unwrap();
        let original = item.0.get_data_component::<CustomNameImpl>().unwrap();
        assert_eq!(
            restored.0.get_data_component::<CustomNameImpl>(),
            Some(original)
        );
        let expected = Cursor::new(payload)
            .get_nbt_with_version(&JavaMinecraftVersion::V_26_3)
            .unwrap()
            .unwrap();
        assert_eq!(original.write_data(), expected);
    }
}

#[test]
fn item_name_wire_roundtrips_literal_styled_and_composite_components() {
    let mut styled = NbtCompound::new();
    styled.put_string("text", "Custom".to_owned());
    styled.put_string("color", "gold".to_owned());
    styled.put("bold", NbtTag::Byte(1));

    let mut first = NbtCompound::new();
    first.put_string("text", "First".to_owned());
    let mut second = NbtCompound::new();
    second.put_string("text", " second".to_owned());
    second.put_string("color", "red".to_owned());

    for tag in [
        NbtTag::Compound({
            let mut literal = NbtCompound::new();
            literal.put_string("text", "Custom".to_owned());
            literal
        }),
        NbtTag::Compound(styled),
        NbtTag::List(vec![NbtTag::Compound(first), NbtTag::Compound(second)]),
    ] {
        let expected = pumpkin_util::text::component_codec::canonical_component(&tag);
        let mut payload = Vec::new();
        tag.clone()
            .serialize(&mut NbtWriteHelperJava::new(&mut payload))
            .unwrap();

        let mut fixture = vec![1];
        fixture
            .write_var_int(&VarInt(i32::from(Item::TRIPWIRE_HOOK.id)))
            .unwrap();
        fixture.extend([1, 0, DataComponent::ItemName.to_id()]);
        fixture.extend(payload);

        let item = ItemStackSerializer::read(&mut Cursor::new(&fixture)).unwrap();
        let name = item
            .0
            .get_data_component::<pumpkin_data::data_component_impl::ItemNameImpl>()
            .unwrap();
        assert_eq!(
            pumpkin_util::text::component_codec::canonical_component(&name.write_data()),
            expected
        );

        let mut written = Vec::new();
        item.write_with_version(&mut written, &JavaMinecraftVersion::V_26_3)
            .unwrap();
        let restored = ItemStackSerializer::read(&mut Cursor::new(written)).unwrap();
        let restored_name = restored
            .0
            .get_data_component::<pumpkin_data::data_component_impl::ItemNameImpl>()
            .unwrap();
        assert_eq!(
            pumpkin_util::text::component_codec::canonical_component(&restored_name.write_data()),
            expected
        );

        let mut saved = NbtCompound::new();
        restored.0.write_item_stack(&mut saved);
        let saved = pumpkin_data::item_stack::ItemStack::read_item_stack(&saved).unwrap();
        assert_eq!(
            pumpkin_util::text::component_codec::canonical_component(
                &saved
                    .get_data_component::<pumpkin_data::data_component_impl::ItemNameImpl>()
                    .unwrap()
                    .write_data()
            ),
            expected
        );
    }
}
