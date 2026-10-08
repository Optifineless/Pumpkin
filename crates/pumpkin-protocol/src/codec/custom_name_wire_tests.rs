use super::*;
use crate::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_data::item::Item;
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
