#![expect(clippy::unwrap_used, reason = "Regression fixtures require valid NBT")]

use super::{
    BannerPatternLayer, BannerPatternsImpl, DataComponentImpl, ItemNameImpl, Rarity, RarityImpl,
    TooltipDisplayImpl,
};
use crate::{data_component::DataComponent, dye_color::DyeColor};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

#[test]
fn old_pr221_native_direct_holder_and_opaque_reference_persistence() {
    let mut direct = NbtCompound::new();
    direct.put_string("asset_id", "custom/banner".into());
    direct.put_string("translation_key", "example.banner".into());
    let mut layer = NbtCompound::new();
    layer.put("pattern", NbtTag::Compound(direct));
    layer.put_string("color", "cyan".into());
    let native = NbtTag::List(vec![NbtTag::Compound(layer)]);
    let value = BannerPatternsImpl::read_data(&native).unwrap();
    assert_eq!(value.layers[0].pattern, "minecraft:custom/banner");
    assert_eq!(
        value.layers[0].translation_key.as_deref(),
        Some("example.banner")
    );
    assert_eq!(value.get_hash() as u32, 0xb1854b2a);
    assert_eq!(
        BannerPatternsImpl::read_data(&value.write_data()),
        Some(value)
    );

    let patterns = BannerPatternsImpl {
        layers: vec![
            BannerPatternLayer {
                pattern: "border".into(),
                color: DyeColor::Black,
                translation_key: None,
            },
            BannerPatternLayer {
                pattern: "pack:unsupported".into(),
                color: DyeColor::Cyan,
                translation_key: None,
            },
        ],
    };
    assert_eq!(patterns.get_hash() as u32, 0x61885d54);
    assert_eq!(
        BannerPatternsImpl::read_data(&patterns.write_data()),
        Some(patterns)
    );
}

#[test]
fn old_pr221_tooltip_codec_defaults_set_order_and_bool_hash() {
    for old in [NbtTag::End, NbtTag::Compound(NbtCompound::new())] {
        let restored = TooltipDisplayImpl::read_data(&old).unwrap();
        assert_eq!(restored, TooltipDisplayImpl);
        assert_eq!(restored.get_hash() as u32, 0xc574b4c8);
        assert_eq!(restored.write_data(), NbtTag::Compound(NbtCompound::new()));
    }
    let value = TooltipDisplayImpl {
        hide_tooltip: true,
        hidden_components: vec![
            DataComponent::BannerPatterns,
            DataComponent::ItemName,
            DataComponent::BannerPatterns,
        ],
    };
    assert_eq!(value.get_hash() as u32, 0xc355e7be);
    let restored = TooltipDisplayImpl::read_data(&value.write_data()).unwrap();
    assert_eq!(restored.hidden_components.len(), 2);
    assert_eq!(
        restored.hidden_components[0].to_name(),
        "minecraft:banner_patterns"
    );
    assert_eq!(
        restored.hidden_components[1].to_name(),
        "minecraft:item_name"
    );
    assert!(restored.hide_tooltip);
    assert_eq!(restored.get_hash(), value.get_hash());
    assert_eq!(restored, value, "hidden components have set equality");
    assert_eq!(
        restored,
        TooltipDisplayImpl {
            hide_tooltip: true,
            hidden_components: vec![DataComponent::ItemName, DataComponent::BannerPatterns],
        }
    );
}

#[test]
fn old_pr221_complete_captain_component_hash_fixtures() {
    // Independent Java 26.3 HashOps fixtures: CRC32C, sorted unsigned map keys.
    let layers = [
        ("rhombus", DyeColor::Cyan),
        ("stripe_bottom", DyeColor::LightGray),
        ("stripe_center", DyeColor::Gray),
        ("border", DyeColor::LightGray),
        ("stripe_middle", DyeColor::Black),
        ("half_horizontal", DyeColor::LightGray),
        ("circle", DyeColor::LightGray),
        ("border", DyeColor::Black),
    ];
    let patterns = BannerPatternsImpl {
        layers: layers
            .into_iter()
            .map(|(pattern, color)| BannerPatternLayer {
                pattern: format!("minecraft:{pattern}"),
                color,
                translation_key: None,
            })
            .collect(),
    };
    assert_eq!(patterns.get_hash() as u32, 0x5349c6bc);
    assert_eq!(
        ItemNameImpl {
            name: "block.minecraft.ominous_banner".into()
        }
        .get_hash() as u32,
        0x0fc5df16
    );
    assert_eq!(
        TooltipDisplayImpl {
            hide_tooltip: false,
            hidden_components: vec![DataComponent::BannerPatterns]
        }
        .get_hash() as u32,
        0x4c2d2266
    );
    assert_eq!(
        RarityImpl {
            rarity: Rarity::Uncommon
        }
        .get_hash() as u32,
        0xb3ccbad2
    );
}

#[test]
fn old_pr221_persistent_validation_and_item_name_preservation_controls() {
    for asset in [
        "bad:UPPER",
        "..:pattern",
        "bad:space here",
        "bad:two:colons",
    ] {
        let mut direct = NbtCompound::new();
        direct.put_string("asset_id", asset.into());
        direct.put_string("translation_key", "key".into());
        let mut layer = NbtCompound::new();
        layer.put("pattern", NbtTag::Compound(direct));
        layer.put_string("color", "cyan".into());
        assert!(
            BannerPatternsImpl::read_data(&NbtTag::List(vec![NbtTag::Compound(layer)])).is_none()
        );
    }
    assert!(ItemNameImpl::read_data(&NbtTag::Compound(NbtCompound::new())).is_none());
    let existing = ItemNameImpl {
        name: "existing.translation.key".into(),
    };
    assert_eq!(
        ItemNameImpl::read_data(&existing.write_data()),
        Some(existing)
    );
}
