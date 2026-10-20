//! Independent 26.3 fixtures for the creative boundary and Java publication.
#![expect(
    clippy::unwrap_used,
    reason = "Independent wire/NBT fixtures must decode before assertions"
)]

use std::{borrow::Cow, collections::BTreeMap, io::Cursor};

use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternsImpl, DataComponentImpl, ItemNameImpl, RarityImpl, TooltipDisplayImpl,
    },
    item_stack::ItemStack,
};
use pumpkin_nbt::{Nbt, NbtCompound, deserializer::NbtReadHelperJava, tag::NbtTag};
use pumpkin_util::version::JavaMinecraftVersion;

use crate::codec::{data_component::DataComponentCodec, item_stack_seralizer::ItemStackSerializer};
use crate::{ServerPacket, java::server::play::SSetCreativeSlot, ser::NetworkReadExt};

// Raid.getBannerComponentPatch, using synchronized registry IDs + 1, then dye IDs.
const CAPTAIN_LAYERS: &[u8] = &[8, 24, 9, 32, 8, 33, 7, 2, 8, 37, 15, 18, 8, 4, 8, 2, 15];
const DIRECT_LAYER: &[u8] = b"\x01\x00\x17minecraft:custom/banner\x0eexample.banner\x09";

fn creative_packet(patterns: &[u8], tooltip: &[u8]) -> Vec<u8> {
    // Slot 36, count 1, white_banner 1417, three additions, zero removals.
    let mut wire = vec![0, 36, 1, 0x89, 0x0b, 3, 0, 74];
    put_varint(&mut wire, patterns.len());
    wire.extend_from_slice(patterns);
    wire.push(18);
    put_varint(&mut wire, tooltip.len());
    wire.extend_from_slice(tooltip);
    wire.extend_from_slice(&[12, 1, 1]); // Rarity body length 1, uncommon ID 1.
    wire
}

fn put_varint(wire: &mut Vec<u8>, mut value: usize) {
    while value >= 128 {
        wire.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    wire.push(value as u8);
}

fn read_creative(wire: &[u8]) -> Result<ItemStack, crate::ser::ReadingError> {
    let mut remaining = wire;
    let packet = SSetCreativeSlot::read_bounded(&mut remaining, &JavaMinecraftVersion::V_26_3)?;
    assert!(
        remaining.is_empty(),
        "creative boundary must consume the packet"
    );
    assert_eq!(packet.slot, 36);
    Ok(packet.clicked_item.to_stack())
}

fn admitted_creative(wire: &[u8]) -> ItemStack {
    let accepted = read_creative(wire);
    assert!(
        accepted.is_ok(),
        "valid native banner Creative fixture must be admitted"
    );
    accepted.unwrap()
}

fn publish(stack: &ItemStack) -> Vec<u8> {
    let mut wire = Vec::new();
    ItemStackSerializer(Cow::Borrowed(stack))
        .write(&mut wire)
        .unwrap();
    wire
}

// Component maps have no ordering contract after an NBT HashMap round trip.
// Frame the actual publication independently, retaining each component's exact bytes.
fn publication_components(stack: &ItemStack) -> BTreeMap<i32, Vec<u8>> {
    let wire = publish(stack);
    let mut input = wire.as_slice();
    assert_eq!(input.get_var_int().unwrap().0, 1);
    assert_eq!(input.get_var_int().unwrap().0, 1417);
    let added = input.get_var_int().unwrap().0;
    assert_eq!(input.get_var_int().unwrap().0, 0);
    let mut components = BTreeMap::new();
    for _ in 0..added {
        let id = input.get_var_int().unwrap().0;
        assert!(matches!(id, 12 | 18 | 74), "unexpected published component");
        let start = input;
        if id == 74 {
            let count = input.get_var_int().unwrap().0;
            for _ in 0..count {
                if input.get_var_int().unwrap().0 == 0 {
                    let _ = input.get_str_bounded(32767).unwrap();
                    let _ = input.get_str_bounded(32767).unwrap();
                }
                let _ = input.get_var_int().unwrap();
            }
        } else if id == 18 {
            let _ = input.get_bool().unwrap();
            let count = input.get_var_int().unwrap().0;
            for _ in 0..count {
                let _ = input.get_var_int().unwrap();
            }
        } else {
            let _ = input.get_var_int().unwrap();
        }
        let body = start[..start.len() - input.len()].to_vec();
        assert!(components.insert(id, body).is_none());
    }
    assert!(
        input.is_empty(),
        "all published component bytes must be consumed"
    );
    components
}

fn expected_components(patterns: &[u8], tooltip: &[u8]) -> BTreeMap<i32, Vec<u8>> {
    BTreeMap::from([
        (74, patterns.to_vec()),
        (18, tooltip.to_vec()),
        (12, vec![1]),
    ])
}

fn disk_roundtrip(stack: &ItemStack) -> (ItemStack, NbtCompound) {
    let mut saved = NbtCompound::new();
    stack.write_item_stack(&mut saved);
    let bytes = Nbt::new(String::new(), saved).write();
    let mut reader = NbtReadHelperJava::new(Cursor::new(bytes.as_ref()));
    let loaded = Nbt::read(&mut reader).unwrap().root_tag;
    let decoded = ItemStack::read_item_stack(&loaded);
    assert!(
        decoded.is_some(),
        "saved banner components must remain loadable"
    );
    (decoded.unwrap(), loaded)
}

#[test]
fn old_pr221_holder_wire_and_persistent_forms_are_native() {
    let stack = admitted_creative(&creative_packet(CAPTAIN_LAYERS, &[0, 2, 74, 9]));
    let banner = stack.get_data_component::<BannerPatternsImpl>().unwrap();
    let expected = [
        ("minecraft:rhombus", "cyan"),
        ("minecraft:stripe_bottom", "light_gray"),
        ("minecraft:stripe_center", "gray"),
        ("minecraft:border", "light_gray"),
        ("minecraft:stripe_middle", "black"),
        ("minecraft:half_horizontal", "light_gray"),
        ("minecraft:circle", "light_gray"),
        ("minecraft:border", "black"),
    ];
    assert_eq!(banner.layers.len(), expected.len());
    for (layer, (pattern, color)) in banner.layers.iter().zip(expected) {
        assert_eq!(
            layer.pattern, pattern,
            "reference holder identity must survive"
        );
        assert_eq!(layer.color.name(), color);
        assert_eq!(layer.translation_key, None);
    }
    assert_eq!(banner.get_hash() as u32, 0x5349c6bc);
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(!tooltip.hide_tooltip);
    assert!(tooltip.hidden_components == [DataComponent::BannerPatterns, DataComponent::ItemName]);
    assert_eq!(
        stack
            .get_data_component::<RarityImpl>()
            .unwrap()
            .rarity
            .to_name(),
        "uncommon",
        "the component after tooltip must remain aligned"
    );
    let mut expected_wire = vec![1, 0x89, 0x0b, 3, 0, 74];
    expected_wire.extend_from_slice(CAPTAIN_LAYERS);
    expected_wire.extend_from_slice(&[18, 0, 2, 74, 9, 12, 1]);
    assert_eq!(
        publish(&stack),
        expected_wire,
        "reference holders require +1"
    );
    let (loaded, _) = disk_roundtrip(&stack);
    assert_eq!(
        publication_components(&loaded),
        expected_components(CAPTAIN_LAYERS, &[0, 2, 74, 9])
    );
}

#[test]
fn old_pr221_direct_holder_zero_payload_and_native_saved_shape() {
    let stack = admitted_creative(&creative_packet(DIRECT_LAYER, &[1, 2, 74, 9]));
    let banner = stack.get_data_component::<BannerPatternsImpl>().unwrap();
    assert_eq!(banner.layers.len(), 1);
    assert_eq!(banner.layers[0].pattern, "minecraft:custom/banner");
    assert_eq!(
        banner.layers[0].translation_key.as_deref(),
        Some("example.banner")
    );
    assert_eq!(banner.layers[0].color.name(), "cyan");
    assert_eq!(banner.get_hash() as u32, 0xb1854b2a);
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(tooltip.hide_tooltip);
    assert_eq!(tooltip.get_hash() as u32, 0xc355e7be);
    let mut expected_wire = vec![1, 0x89, 0x0b, 3, 0, 74];
    expected_wire.extend_from_slice(DIRECT_LAYER);
    expected_wire.extend_from_slice(&[18, 1, 2, 74, 9, 12, 1]);
    assert_eq!(
        publish(&stack),
        expected_wire,
        "direct fields precede the dye"
    );
    let abbreviated = b"\x01\x00\x0dcustom/banner\x0eexample.banner\x09";
    let canonicalized = admitted_creative(&creative_packet(abbreviated, &[1, 2, 74, 9]));
    assert_eq!(
        publish(&canonicalized),
        expected_wire,
        "direct ingress canonicalizes aliases"
    );
    let (loaded, saved) = disk_roundtrip(&stack);
    let layers = saved
        .get_compound("components")
        .unwrap()
        .get_list("minecraft:banner_patterns")
        .unwrap();
    let native = layers[0]
        .extract_compound()
        .unwrap()
        .get_compound("pattern");
    assert!(
        native.is_some(),
        "direct holders must use the native nested compound"
    );
    let pattern = native.unwrap();
    assert_eq!(
        pattern.get_string("asset_id"),
        Some("minecraft:custom/banner")
    );
    assert_eq!(
        pattern.get_string("translation_key"),
        Some("example.banner")
    );
    assert_eq!(
        publication_components(&loaded),
        expected_components(DIRECT_LAYER, &[1, 2, 74, 9]),
        "valid native direct stacks survive load"
    );
}

#[test]
fn old_pr221_vanilla_nested_direct_holder_loads_without_flattening() {
    let mut pattern = NbtCompound::new();
    pattern.put_string("asset_id", "custom/banner".into());
    pattern.put_string("translation_key", "example.banner".into());
    let mut layer = NbtCompound::new();
    layer.put_compound("pattern", pattern);
    layer.put_string("color", "cyan".into());
    let mut components = NbtCompound::new();
    components.put_list("minecraft:banner_patterns", vec![NbtTag::Compound(layer)]);
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:white_banner".into());
    saved.put_int("count", 1);
    saved.put_compound("components", components);
    let loaded = ItemStack::read_item_stack(&saved);
    assert!(loaded.is_some(), "valid native saved banner must load");
    let stack = loaded.unwrap();
    let mut expected = vec![1, 0x89, 0x0b, 1, 0, 74];
    expected.extend_from_slice(DIRECT_LAYER);
    assert_eq!(
        publish(&stack),
        expected,
        "abbreviated valid assets canonicalize"
    );
    let (loaded, written) = disk_roundtrip(&stack);
    assert_eq!(publish(&loaded), expected);
    let layers = written
        .get_compound("components")
        .unwrap()
        .get_list("minecraft:banner_patterns")
        .unwrap();
    assert!(
        layers[0]
            .extract_compound()
            .unwrap()
            .get_compound("pattern")
            .is_some()
    );
}

#[test]
fn old_pr221_unknown_saved_layer_projects_without_changing_authoritative_data() {
    let mut known = NbtCompound::new();
    known.put_string("pattern", "border".into());
    known.put_string("color", "black".into());
    let mut unknown = NbtCompound::new();
    unknown.put_string("pattern", "example:unsupported_saved_pattern".into());
    unknown.put_string("color", "cyan".into());
    let mut components = NbtCompound::new();
    components.put_list(
        "minecraft:banner_patterns",
        vec![known.into(), unknown.into()],
    );
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:white_banner".into());
    saved.put_int("count", 1);
    saved.put_compound("components", components);
    let loaded = ItemStack::read_item_stack(&saved);
    assert!(loaded.is_some(), "valid native saved banner must load");
    let stack = loaded.unwrap();
    assert_eq!(publish(&stack), [1, 0x89, 0x0b, 1, 0, 74, 1, 2, 15]);
    assert_eq!(
        stack
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .get_hash() as u32,
        0x61885d54
    );
    let (loaded, _) = disk_roundtrip(&stack);
    let banner = loaded.get_data_component::<BannerPatternsImpl>().unwrap();
    assert_eq!(
        banner.layers.len(),
        2,
        "publication must conserve stored layers"
    );
    assert_eq!(
        banner.layers[1].pattern,
        "example:unsupported_saved_pattern"
    );
    assert_eq!(publish(&loaded), publish(&stack));
}

#[test]
fn old_pr221_holder_and_tooltip_ingress_bounds_reject_malformed_data() {
    for (reason, patterns) in [
        ("negative layer count", vec![0xff, 0xff, 0xff, 0xff, 0x0f]),
        ("oversized layer count", vec![0xff, 0xff, 0xff, 0xff, 7]),
        ("negative holder", vec![1, 0xff, 0xff, 0xff, 0xff, 0x0f, 9]),
        ("out of range holder", vec![1, 44, 9]),
        ("negative dye", vec![1, 2, 0xff, 0xff, 0xff, 0xff, 0x0f]),
        ("out of range dye", vec![1, 2, 16]),
        ("truncated reference", vec![1, 2]),
        ("truncated direct", vec![1, 0, 3, b'a']),
        ("bad asset", b"\x01\x00\x07Bad:foo\x01x\x09".to_vec()),
        ("bad namespace", b"\x01\x00\x06..:foo\x01x\x09".to_vec()),
        ("oversized asset", vec![1, 0, 0xff, 0xff, 0xff, 0xff, 7]),
        (
            "oversized translation",
            b"\x01\x00\x03foo\xff\xff\xff\xff\x07".to_vec(),
        ),
        (
            "invalid translation UTF8",
            b"\x01\x00\x03foo\x01\xff\x09".to_vec(),
        ),
        ("trailing component byte", vec![1, 2, 9, 0]),
    ] {
        assert!(
            read_creative(&creative_packet(&patterns, &[0, 0])).is_err(),
            "{reason}"
        );
    }
    for tooltip in [
        vec![0, 0xff, 0xff, 0xff, 0xff, 0x0f],
        vec![0, 0xff, 0xff, 0xff, 0xff, 7],
        vec![0, 1, 0xff, 0xff, 0xff, 0xff, 0x0f],
        vec![0, 1, 0xff, 1],
        vec![0, 2, 74],
        vec![0, 0, 0],
    ] {
        assert!(read_creative(&creative_packet(&[1, 2, 9], &tooltip)).is_err());
    }
    assert!(read_creative(&creative_packet(&[1, 2, 9], &[0, 0])).is_ok());
}

#[test]
fn old_pr221_tooltip_duplicate_components_keep_set_order() {
    let stack = admitted_creative(&creative_packet(&[1, 2, 9], &[0, 3, 74, 9, 74]));
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(tooltip.hidden_components == [DataComponent::BannerPatterns, DataComponent::ItemName]);
    let mut wire = Vec::new();
    tooltip.serialize(&mut wire).unwrap();
    assert_eq!(wire, [0, 2, 74, 9]);
    let (loaded, _) = disk_roundtrip(&stack);
    assert_eq!(
        publication_components(&loaded),
        expected_components(&[1, 2, 9], &[0, 2, 74, 9])
    );
}

#[test]
fn old_pr221_original_malformed_item_name_stays_safely_normalized() {
    // Original B1 Creative packet: slot36, stone, ITEM_NAME = unnamed empty compound.
    let wire = [0, 36, 1, 1, 1, 0, 9, 2, 10, 0];
    let stack = read_creative(&wire).unwrap();
    let name = stack.get_data_component::<ItemNameImpl>().unwrap();
    assert_eq!(name.name.as_ref(), "");
    let (loaded, _) = disk_roundtrip(&stack);
    let emitted = loaded
        .get_data_component::<ItemNameImpl>()
        .unwrap()
        .write_data();
    assert_eq!(
        emitted.extract_compound().unwrap().get_string("translate"),
        Some("")
    );
    assert_eq!(publish(&loaded), publish(&stack));
}

#[test]
fn old_pr221_legacy_tooltip_end_normalizes_without_losing_following_components() {
    // The historical unit form is representable in the saved in-memory NBT model.
    // This does not claim recovery of malformed historical disk tag0+name bytes.
    let mut components = NbtCompound::new();
    components.put("minecraft:tooltip_display", NbtTag::End);
    components.put_string("minecraft:rarity", "uncommon".into());
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:white_banner".into());
    saved.put_int("count", 1);
    saved.put_compound("components", components);
    let loaded = ItemStack::read_item_stack(&saved);
    assert!(
        loaded.is_some(),
        "legacy unit tooltip must preserve the saved stack"
    );
    let stack = loaded.unwrap();
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(!tooltip.hide_tooltip && tooltip.hidden_components.is_empty());
    let expected = BTreeMap::from([(18, vec![0, 0]), (12, vec![1])]);
    assert_eq!(publication_components(&stack), expected);
    let (reloaded, saved) = disk_roundtrip(&stack);
    let normalized = saved
        .get_compound("components")
        .unwrap()
        .get_compound("minecraft:tooltip_display");
    assert!(
        normalized.is_some_and(NbtCompound::is_empty),
        "legacy tooltip writes native empty compound"
    );
    assert_eq!(publication_components(&reloaded), expected);
    assert_eq!(
        reloaded
            .get_data_component::<RarityImpl>()
            .unwrap()
            .rarity
            .to_name(),
        "uncommon"
    );
}
