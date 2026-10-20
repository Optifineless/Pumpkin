#![expect(clippy::unwrap_used, reason = "Protocol-derived fixed-LE fixtures")]

use super::*;
use pumpkin_data::item::Item;

#[test]
fn bedrock_unencodable_item_nbt_never_emits_partial_user_data() {
    let mut descriptor = NetworkItemDescriptor::from(&ItemStack::new(1, &Item::DIAMOND));
    descriptor
        .nbt_data
        .root_tag
        .put_string("Name", "x".repeat(32768));
    let mut bytes = Vec::new();
    assert!(descriptor.write_user_data(&mut bytes).is_err());
    assert!(
        bytes.is_empty(),
        "no user-data length or partial NBT can be published"
    );
}

#[test]
fn bedrock_plain_banner_uses_fixed_le_empty_patterns_golden() {
    let banner = ItemStack::new(1, &Item::WHITE_BANNER);
    let descriptor = NetworkItemStackDescriptor::from(&banner);
    // -1/version1; named compound; list "Patterns"; End elements; fixed i32 zero;
    // compound End; two fixed i32 filter counts. No item-specific name is set.
    assert_eq!(
        descriptor.extra_data,
        [
            0xff, 0xff, 1, 10, 0, 0, 9, 8, 0, b'P', b'a', b't', b't', b'e', b'r', b'n', b's', 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]
    );
}

#[test]
fn bedrock_item_readers_accept_fixed_le_and_preserve_filter_shield_suffixes() {
    // Protocol-derived NBT bytes, independent of either Pumpkin NBT writer.
    let mut user_data = vec![
        0xff, 0xff, 1, 10, 0, 0, 10, 7, 0, b'd', b'i', b's', b'p', b'l', b'a', b'y', 8, 4, 0, b'N',
        b'a', b'm', b'e', 6, 0, 0xc3, 0xa9, 0xf0, 0x9f, 0x8e, 0xa8, 0, 0,
    ];
    user_data.extend_from_slice(&1i32.to_le_bytes());
    user_data.extend_from_slice(&[0, 5, b's', b't', b'o', b'n', b'e']);
    user_data.extend_from_slice(&0i32.to_le_bytes());
    user_data.extend_from_slice(&123456789i64.to_le_bytes());
    let mut bytes = Vec::new();
    BedrockItem::SHIELD.id.write(&mut bytes).unwrap();
    1u16.write(&mut bytes).unwrap();
    VarUInt(0).write(&mut bytes).unwrap();
    false.write(&mut bytes).unwrap();
    VarUInt(0).write(&mut bytes).unwrap();
    VarUInt(user_data.len() as u32).write(&mut bytes).unwrap();
    bytes.extend_from_slice(&user_data);
    let mut descriptor_reader = bytes.as_slice();
    let descriptor = NetworkItemDescriptor::read(&mut descriptor_reader).unwrap();
    let mut wrapper_reader = bytes.as_slice();
    let wrapper = ItemStackWrapper::read(&mut wrapper_reader).unwrap();
    assert!(descriptor_reader.is_empty() && wrapper_reader.is_empty());
    for (nbt, place, destroy, tick) in [
        (
            &descriptor.nbt_data,
            &descriptor.place_on_blocks,
            &descriptor.destroy_blocks,
            descriptor.shield_blocking_tick,
        ),
        (
            &wrapper.nbt_data,
            &wrapper.place_on_blocks,
            &wrapper.destroy_blocks,
            wrapper.shield_blocking_tick,
        ),
    ] {
        assert_eq!(
            nbt.get_compound("display").unwrap().get_string("Name"),
            Some("é🎨")
        );
        assert_eq!(place, &["stone"]);
        assert!(destroy.is_empty());
        assert_eq!(tick, 123456789);
    }
    let mut encoded = Vec::new();
    wrapper.write(&mut encoded).unwrap();
    assert_eq!(
        encoded, bytes,
        "fixed-LE item and suffix bytes round trip exactly"
    );
}
