use super::*;
use crate::ClientPacket;
use crate::codec::item_stack_seralizer::ItemStackSerializer;
use crate::java::client::play::CSetEquipment;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::{
    BannerPatternLayer, BannerPatternsImpl, TooltipDisplayImpl,
};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_data::registry::Registry;
use pumpkin_util::version::JavaMinecraftVersion;

const OMINOUS_LAYERS: [(&str, i32, pumpkin_data::dye_color::DyeColor, i32); 8] = [
    (
        "minecraft:rhombus",
        23,
        pumpkin_data::dye_color::DyeColor::Cyan,
        9,
    ),
    (
        "minecraft:stripe_bottom",
        31,
        pumpkin_data::dye_color::DyeColor::LightGray,
        8,
    ),
    (
        "minecraft:stripe_center",
        32,
        pumpkin_data::dye_color::DyeColor::Gray,
        7,
    ),
    (
        "minecraft:border",
        1,
        pumpkin_data::dye_color::DyeColor::LightGray,
        8,
    ),
    (
        "minecraft:stripe_middle",
        36,
        pumpkin_data::dye_color::DyeColor::Black,
        15,
    ),
    (
        "minecraft:half_horizontal",
        17,
        pumpkin_data::dye_color::DyeColor::LightGray,
        8,
    ),
    (
        "minecraft:circle",
        3,
        pumpkin_data::dye_color::DyeColor::LightGray,
        8,
    ),
    (
        "minecraft:border",
        1,
        pumpkin_data::dye_color::DyeColor::Black,
        15,
    ),
];

fn ominous_patterns() -> BannerPatternsImpl {
    BannerPatternsImpl {
        layers: OMINOUS_LAYERS
            .iter()
            .map(|(pattern, _, color, _)| BannerPatternLayer {
                pattern: (*pattern).to_owned(),
                translation_key: None,
                color: *color,
            })
            .collect(),
    }
}

fn equipment_packet_bytes(stack: ItemStack) -> Vec<u8> {
    let packet = CSetEquipment::new(VarInt(42), vec![(5, ItemStackSerializer::from(stack))]);
    let mut bytes = Vec::new();
    packet
        .write_packet_data(&mut bytes, &CURRENT_MC_VERSION)
        .unwrap();
    bytes
}

fn read_equipment_banner_pattern_layers(bytes: &[u8]) -> Vec<(i32, i32)> {
    let mut read = bytes;
    assert_eq!(read.get_var_int().unwrap(), VarInt(42));
    assert_eq!(read.get_u8().unwrap(), 5);
    assert_eq!(read.get_var_int().unwrap(), VarInt(1));
    let _item_id = read.get_var_int().unwrap();
    let to_add = read.get_var_int().unwrap().0;
    let to_remove = read.get_var_int().unwrap().0;
    let mut layers = Vec::new();

    for _ in 0..to_add {
        let raw_component_id = u8::try_from(read.get_var_int().unwrap().0).unwrap();
        let component = DataComponent::try_from_id(raw_component_id).unwrap();
        if component == DataComponent::BannerPatterns {
            let layer_count = read.get_var_int().unwrap().0;
            for _ in 0..layer_count {
                let pattern_id = read.get_var_int().unwrap().0;
                let color_id = read.get_var_int().unwrap().0;
                layers.push((pattern_id, color_id));
            }
        } else {
            deserialize(component, &mut read).unwrap();
        }
    }
    for _ in 0..to_remove {
        let _ = read.get_var_int().unwrap();
    }
    assert!(read.is_empty());
    layers
}

fn decode_equipment_stack(bytes: &[u8]) -> ItemStackSerializer<'static> {
    let mut read = bytes;
    let _entity_id = read.get_var_int().unwrap();
    let _slot = read.get_u8().unwrap();
    ItemStackSerializer::read_with_version(&mut read, &CURRENT_MC_VERSION).unwrap()
}

#[test]
fn banner_patterns_equipment_packet_uses_synced_registry_ids() {
    assert_eq!(CURRENT_MC_VERSION, JavaMinecraftVersion::V_26_3);
    let registry = Registry::get_synced(CURRENT_MC_VERSION)
        .into_iter()
        .find(|registry| registry.registry_id == "minecraft:banner_pattern")
        .unwrap();

    for (name, expected_id, _, _) in OMINOUS_LAYERS {
        let actual_id = registry
            .registry_entries
            .iter()
            .position(|entry| entry.entry_id == name)
            .unwrap();
        assert_eq!(i32::try_from(actual_id).unwrap(), expected_id, "{name}");
    }

    let mut stack = ItemStack::new(1, &Item::WHITE_BANNER);
    stack.set_data_component(ominous_patterns());
    let bytes = equipment_packet_bytes(stack);
    // ByteBufCodecs.holder: a registry reference is written as id + 1 (0 is an inline value).
    assert_eq!(
        read_equipment_banner_pattern_layers(&bytes),
        OMINOUS_LAYERS
            .iter()
            .map(|(_, pattern_id, _, color_id)| (*pattern_id + 1, *color_id))
            .collect::<Vec<_>>()
    );
}

#[test]
fn banner_patterns_codec_round_trips_names_and_colors() {
    let patterns = ominous_patterns();
    let mut bytes = Vec::new();
    patterns.serialize(&mut bytes).unwrap();

    let mut read = bytes.as_slice();
    let decoded = BannerPatternsImpl::deserialize(&mut read).unwrap();
    assert!(read.is_empty());
    assert_eq!(decoded, patterns);
}

#[test]
fn banner_patterns_tooltip_display_codec_preserves_hidden_component_state() {
    let tooltip = TooltipDisplayImpl {
        hide_tooltip: false,
        hidden_components: vec![DataComponent::BannerPatterns],
    };
    let mut bytes = Vec::new();
    tooltip.serialize(&mut bytes).unwrap();
    // TooltipDisplay.STREAM_CODEC: one hideTooltip bool, then the component type collection.
    assert_eq!(
        bytes,
        vec![0, 1, i32::from(DataComponent::BannerPatterns.to_id()) as u8]
    );

    let mut read = bytes.as_slice();
    assert_eq!(TooltipDisplayImpl::deserialize(&mut read).unwrap(), tooltip);
    assert!(read.is_empty());
}

#[test]
fn banner_patterns_plain_white_and_unrelated_pattern_controls_are_preserved() {
    let plain = ItemStack::new(1, &Item::WHITE_BANNER);
    let plain_bytes = equipment_packet_bytes(plain);
    assert!(read_equipment_banner_pattern_layers(&plain_bytes).is_empty());
    let decoded_plain = decode_equipment_stack(&plain_bytes);
    assert!(decoded_plain.0.patch.is_empty());
    assert!(
        decoded_plain
            .0
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .is_empty()
    );

    let custom_layer = BannerPatternLayer {
        pattern: "minecraft:flower".to_owned(),
        translation_key: None,
        color: pumpkin_data::dye_color::DyeColor::Red,
    };
    let mut patterned = ItemStack::new(1, &Item::WHITE_BANNER);
    patterned.set_data_component(BannerPatternsImpl {
        layers: vec![custom_layer.clone()],
    });
    let bytes = equipment_packet_bytes(patterned.clone());
    let flower_id = Registry::get_synced(CURRENT_MC_VERSION)
        .into_iter()
        .find(|registry| registry.registry_id == "minecraft:banner_pattern")
        .unwrap()
        .registry_entries
        .iter()
        .position(|entry| entry.entry_id == "minecraft:flower")
        .unwrap();
    assert_eq!(
        read_equipment_banner_pattern_layers(&bytes),
        vec![(i32::try_from(flower_id).unwrap() + 1, 14)]
    );
    let decoded = decode_equipment_stack(&bytes);
    assert_eq!(
        decoded
            .0
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers,
        vec![custom_layer]
    );
}

#[test]
fn banner_patterns_unknown_saved_layer_is_skipped_without_losing_supported_layers() {
    let unknown = BannerPatternLayer {
        pattern: "murgicraft:unknown_pattern".to_owned(),
        translation_key: None,
        color: pumpkin_data::dye_color::DyeColor::Red,
    };
    let known = BannerPatternLayer {
        pattern: "minecraft:flower".to_owned(),
        translation_key: None,
        color: pumpkin_data::dye_color::DyeColor::Blue,
    };
    let mut stack = ItemStack::new(1, &Item::WHITE_BANNER);
    stack.set_data_component(BannerPatternsImpl {
        layers: vec![unknown.clone(), known.clone()],
    });
    let bytes = equipment_packet_bytes(stack.clone());
    let flower_id = Registry::get_synced(CURRENT_MC_VERSION)
        .into_iter()
        .find(|registry| registry.registry_id == "minecraft:banner_pattern")
        .unwrap()
        .registry_entries
        .iter()
        .position(|entry| entry.entry_id == "minecraft:flower")
        .unwrap();
    assert_eq!(
        read_equipment_banner_pattern_layers(&bytes),
        vec![(
            i32::try_from(flower_id).unwrap() + 1,
            i32::from(pumpkin_data::dye_color::DyeColor::Blue.id())
        )]
    );
    assert_eq!(
        stack
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers,
        vec![unknown, known.clone()]
    );
    let decoded = decode_equipment_stack(&bytes);
    assert_eq!(
        decoded
            .0
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers,
        vec![known]
    );
}

#[test]
fn banner_patterns_decode_and_encode_vanilla_direct_holder_fixture() {
    // BannerPattern.DIRECT_STREAM_CODEC: asset ID then translation key, followed by dye color.
    let fixture = [
        1, // one layer
        0, // direct holder
        17, b'm', b'i', b'n', b'e', b'c', b'r', b'a', b'f', b't', b':', b'c', b'r', b'e', b'e',
        b'p', b'e', b'r', 30, b'b', b'l', b'o', b'c', b'k', b'.', b'm', b'i', b'n', b'e', b'c',
        b'r', b'a', b'f', b't', b'.', b'b', b'a', b'n', b'n', b'e', b'r', b'.', b'c', b'r', b'e',
        b'e', b'p', b'e', b'r', 14,
    ];
    let mut read = fixture.as_slice();
    let decoded = BannerPatternsImpl::deserialize(&mut read).unwrap();
    assert!(read.is_empty());
    assert_eq!(
        decoded.layers,
        vec![BannerPatternLayer {
            pattern: "minecraft:creeper".to_owned(),
            translation_key: Some("block.minecraft.banner.creeper".to_owned()),
            color: pumpkin_data::dye_color::DyeColor::Red,
        }]
    );
    let mut encoded = Vec::new();
    decoded.serialize(&mut encoded).unwrap();
    assert_eq!(encoded, fixture);
}

#[test]
fn banner_patterns_reject_negative_holders() {
    for holder in [-1, i32::MIN] {
        let mut bytes = Vec::new();
        bytes.write_var_int(&VarInt(1)).unwrap();
        bytes.write_var_int(&VarInt(holder)).unwrap();
        bytes.write_var_int(&VarInt(0)).unwrap();
        let mut read = bytes.as_slice();
        assert!(
            BannerPatternsImpl::deserialize(&mut read).is_err(),
            "{holder}"
        );
    }
}
