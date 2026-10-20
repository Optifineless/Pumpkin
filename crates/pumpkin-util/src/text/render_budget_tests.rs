use super::{TextComponent, TextContent};
use crate::{
    translation::{Locale, get_translation_text, translation_to_pretty},
    version::JavaMinecraftVersion,
};

const LIMIT: usize = 32 * 1024;

fn repeated(depth: usize, custom: bool) -> TextComponent {
    let mut component = TextComponent::text("A");
    for _ in 0..depth {
        component = TextComponent::from_content(if custom {
            TextContent::Custom {
                key: "minecraft:translation.test.complex".into(),
                locale: Locale::EnUs,
                with: vec![component.0],
            }
        } else {
            TextContent::Translate {
                translate: "translation.test.complex".into(),
                bedrock_translate: None,
                with: vec![component.0],
            }
        });
    }
    component
}

fn bounded(output: &str) {
    assert!(!output.is_empty());
    assert!(output.len() <= LIMIT, "Rendered {} bytes", output.len());
}

// S5 expands to only 77,751 bytes before the fix, so every negative control is safe.
#[test]
fn render_budget_native_console() {
    bounded(&repeated(5, false).to_pretty_console());
}

#[test]
fn render_budget_translation_pretty() {
    bounded(&translation_to_pretty(
        "minecraft:translation.test.complex",
        Locale::EnUs,
        vec![repeated(4, false).0],
    ));
}

#[test]
fn render_budget_translation_text() {
    bounded(&get_translation_text(
        "minecraft:translation.test.complex",
        Locale::EnUs,
        vec![repeated(4, false).0],
    ));
}

#[test]
fn render_budget_java_legacy() {
    bounded(&repeated(5, false).to_legacy_string(Locale::EnUs));
}

#[test]
fn render_budget_bedrock_legacy() {
    bounded(&repeated(5, true).0.to_bedrock_legacy(Locale::EnUs));
}

#[test]
fn render_budget_bedrock_raw_forest() {
    let component =
        TextComponent::text("😀".repeat(5_000)).add_child(TextComponent::text("😀".repeat(5_000)));
    bounded(&component.0.to_bedrock_string());
}

#[test]
fn render_budget_plain_forest() {
    bounded(&repeated(5, false).add_child(repeated(5, false)).get_text());
}

fn text_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                if key == "text" {
                    value.as_str().map_or(0, str::len)
                } else {
                    text_bytes(value)
                }
            })
            .sum(),
        serde_json::Value::Array(values) => values.iter().map(text_bytes).sum(),
        serde_json::Value::String(value) => value.len(),
        _ => 0,
    }
}

#[test]
fn render_budget_custom_materialization() {
    let json = repeated(5, true).to_json_value_for_version(&JavaMinecraftVersion::V_26_3);
    assert!(
        text_bytes(&json) <= LIMIT,
        "Materialized {} bytes",
        text_bytes(&json)
    );
}

#[test]
fn render_budget_ordinary_translations_unchanged() {
    let expected = "Holder has 42 Kills";
    let component = TextComponent::from_content(TextContent::Translate {
        translate: "commands.scoreboard.players.get.success".into(),
        bedrock_translate: Some("commands.scoreboard.players.get.success".into()),
        with: ["Holder", "42", "Kills"]
            .map(|text| TextComponent::text(text).0)
            .into(),
    });
    assert_eq!(component.clone().get_text(), expected);
    assert_eq!(component.clone().to_pretty_console(), expected);
    assert_eq!(component.to_legacy_string(Locale::EnUs), expected);
    assert_eq!(
        translation_to_pretty(
            "minecraft:commands.scoreboard.players.get.success",
            Locale::EnUs,
            ["Holder", "42", "Kills"]
                .map(|text| TextComponent::text(text).0)
                .into()
        ),
        expected
    );
}

#[test]
fn render_budget_empty_expansion_stops_before_later_siblings() {
    crate::translation::add_translation("render_budget", "empty", "%1$s%1$s", Locale::EnUs);
    let mut component = TextComponent::empty();
    for _ in 0..18 {
        component = TextComponent::from_content(TextContent::Custom {
            key: "render_budget:empty".into(),
            locale: Locale::EnUs,
            with: vec![component.0],
        });
    }
    let component = component.add_child(TextComponent::text("unvisited"));
    assert_eq!(component.clone().get_text(), "");
    assert_eq!(component.clone().to_pretty_console(), "");
    assert_eq!(component.to_legacy_string(Locale::EnUs), "");
    assert_eq!(component.0.to_bedrock_legacy(Locale::EnUs), "");
    assert_eq!(component.0.to_translated().get_text(Locale::EnUs), "");
}

#[test]
fn render_budget_hover_and_visible_text_share_materialization_budget() {
    use super::hover::HoverEvent;
    let component = TextComponent::text("visible").hover_event(HoverEvent::ShowText {
        value: vec![repeated(5, true).0; 2],
    });
    let value = component.to_json_for_version(&JavaMinecraftVersion::V_26_3);
    assert!(value.len() <= LIMIT, "Materialized {} bytes", value.len());
    assert!(value.contains("visible"));
}

#[test]
fn render_budget_utf8_formatting_links_and_ordinary_custom_text() {
    use super::{
        click::ClickEvent,
        color::{Color, NamedColor},
    };
    let value = TextComponent::text("😀".repeat(10_000))
        .color(Color::Named(NamedColor::Red))
        .bold()
        .click_event(ClickEvent::OpenUrl {
            url: "https://example.org/".into(),
        });
    bounded(&value.clone().to_pretty_console());
    bounded(&value.to_legacy_string(Locale::EnUs));
    bounded(&value.0.to_bedrock_legacy(Locale::EnUs));
    let ordinary = TextComponent::text("hello").click_event(ClickEvent::OpenUrl {
        url: "https://example.org/".into(),
    });
    assert_eq!(
        ordinary.to_pretty_console(),
        "\x1b]8;;https://example.org/\x1b\\hello\x1b]8;;\x1b\\"
    );
    crate::translation::add_translation("render_budget", "ordinary", "%% %s + %2$s!", Locale::EnUs);
    let ordinary = TextComponent::from_content(TextContent::Custom {
        key: "render_budget:ordinary".into(),
        locale: Locale::EnUs,
        with: ["Alice", "Bob"]
            .map(|text| TextComponent::text(text).0)
            .into(),
    });
    assert_eq!(ordinary.clone().get_text(), "% Alice + Bob!");
    assert_eq!(
        ordinary.0.to_translated().get_text(Locale::EnUs),
        "% Alice + Bob!"
    );
}

#[test]
fn render_budget_opaque_substitutions_are_charged_before_cloning() {
    use super::{ProfileNbt, component_codec};
    for content in [
        TextContent::Opaque {
            component: ProfileNbt({
                let mut raw = pumpkin_nbt::NbtCompound::new();
                raw.put_string("text", "A".repeat(5_000));
                raw
            }),
        },
        TextContent::Translatable {
            component: ProfileNbt({
                let mut raw = pumpkin_nbt::NbtCompound::new();
                raw.put_string("translate", "raw.key".into());
                raw.put_string("fallback", "A".repeat(5_000));
                raw
            }),
        },
        TextContent::PlayerSprite {
            type_name: "object".into(),
            profile: ProfileNbt({
                let mut raw = pumpkin_nbt::NbtCompound::new();
                raw.put_string("name", "A".repeat(5_000));
                raw
            }),
            hat: true,
        },
    ] {
        let original = TextComponent::from_content(content);
        let raw = component_codec::write_component(&original);
        let mut component = original.clone();
        for _ in 0..2 {
            component = TextComponent::from_content(TextContent::Custom {
                key: "minecraft:translation.test.complex".into(),
                locale: Locale::EnUs,
                with: vec![component.0],
            });
        }
        let encoded = component.to_json_for_version(&JavaMinecraftVersion::V_26_3);
        assert!(encoded.len() <= LIMIT, "Cloned {} bytes", encoded.len());
        assert_eq!(
            component_codec::write_component(&original),
            raw,
            "Storage codec acquired display truncation"
        );
    }
}

#[test]
fn render_budget_exported_substitutions_bound_cloned_payloads() {
    let (values, ranges) = crate::translation::reorder_substitutions(
        "%1$s%1$s%1$s%1$s%1$s%1$s",
        vec![TextComponent::text("A".repeat(10_000)).0],
    );
    assert_eq!(values.len(), ranges.len());
    assert!(values.len() < 6);
    assert!(
        values
            .into_iter()
            .map(|value| value.get_text(Locale::EnUs).len())
            .sum::<usize>()
            <= LIMIT
    );
}

#[test]
fn render_budget_native_storage_encoders_remain_lossless() {
    let text = "A".repeat(40_000);
    let literal = TextComponent::text(text.clone());
    // Entity::write_nbt saves CustomName through this existing public encoder.
    let saved = literal.to_nbt_tag_for_version(&JavaMinecraftVersion::V_26_3);
    assert_eq!(saved.extract_string().map(str::len), Some(text.len()));
    // TextDisplayEntity::write_custom_nbt saves text through generic serde.
    let json = serde_json::to_value(&literal).unwrap();
    assert_eq!(json["text"].as_str(), Some(text.as_str()));
    let mixed = literal
        .clone()
        .add_child(TextComponent::from_content(TextContent::Custom {
            key: "x".into(),
            with: vec![],
            locale: Locale::EnUs,
        }));
    let json = serde_json::to_value(&mixed).unwrap();
    assert_eq!(json["text"].as_str().map(str::len), Some(text.len()));
    assert_eq!(json["extra"][0]["text"], "x");
    let saved = mixed.to_nbt_tag_for_version(&JavaMinecraftVersion::V_26_3);
    assert_eq!(
        saved.extract_compound().unwrap().get_string("text"),
        Some(text.as_str())
    );
    let native = TextComponent::from_content(TextContent::Translate {
        translate: "commands.seed.success".into(),
        bedrock_translate: None,
        with: vec![literal.0],
    });
    assert_eq!(
        native.to_nbt_tag_for_version(&JavaMinecraftVersion::V_26_3),
        super::component_codec::write_component(&native)
    );
}

#[test]
fn render_budget_structured_style_payloads_are_atomic() {
    use super::{click::ClickEvent, hover::HoverEvent};
    let payload = "minecraft:polished_blackstone_brick_slab";
    let custom = TextComponent::from_content(TextContent::Custom {
        key: "x".into(),
        with: vec![],
        locale: Locale::EnUs,
    });
    let near = TextComponent::text("A".repeat(32_360)).add_child(custom);
    for event in [
        ClickEvent::OpenUrl {
            url: "https://example.org/complete/resource".into(),
        },
        ClickEvent::OpenFile {
            path: payload.into(),
        },
        ClickEvent::RunCommand {
            command: payload.into(),
        },
        ClickEvent::SuggestCommand {
            command: payload.into(),
        },
        ClickEvent::CopyToClipboard {
            value: payload.into(),
        },
    ] {
        let output = near.clone().click_event(event.clone()).0.to_translated();
        assert!(output.style.click_event.is_none_or(|value| value == event));
    }
    let mut font = near.clone();
    font.0.style.font = Some(payload.into());
    assert!(
        font.0
            .to_translated()
            .style
            .font
            .is_none_or(|value| value == payload)
    );
    let event = HoverEvent::ShowItem {
        id: payload.into(),
        count: Some(1),
    };
    assert!(
        near.clone()
            .hover_event(event.clone())
            .0
            .to_translated()
            .style
            .hover_event
            .is_none_or(|value| value == event)
    );
    let event = HoverEvent::ShowText {
        value: vec![TextComponent::text("A".repeat(1_000)).0],
    };
    assert!(
        near.hover_event(event.clone())
            .0
            .to_translated()
            .style
            .hover_event
            .is_none_or(|value| value == event)
    );
}
