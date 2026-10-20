use super::{TextComponent, component_codec};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};

#[test]
fn review3_shared_decoder_preserves_translation_arguments_and_fallback() {
    for arguments in [None, Some(NbtTag::List(vec![NbtTag::Int(1)]))] {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", "unknown.key".into());
        nbt.put_string("fallback", "Key %s".into());
        if let Some(arguments) = arguments {
            nbt.put("with", arguments);
        }
        let tag = NbtTag::Compound(nbt);
        let decoded = TextComponent::from_nbt(&tag);
        assert_eq!(
            decoded.0.to_nbt_compound(),
            tag.extract_compound().unwrap().clone()
        );
    }
}

fn component(fields: &[(&str, NbtTag)]) -> NbtTag {
    let mut data = NbtCompound::new();
    for (field, value) in fields {
        data.put(field, value.clone());
    }
    NbtTag::Compound(data)
}

fn string(value: &str) -> NbtTag {
    NbtTag::String(value.into())
}

#[test]
fn component_validation_rejects_malformed_content_and_strict_children() {
    let empty = component(&[]);
    for invalid in [
        empty.clone(),
        NbtTag::Int(4),
        NbtTag::List(vec![]),
        NbtTag::List(vec![string("valid"), empty.clone()]),
        component(&[("text", NbtTag::Int(1))]),
        component(&[("type", string("unknown")), ("text", string("valid"))]),
        component(&[("type", string("nbt")), ("text", string("valid"))]),
        component(&[("type", NbtTag::Int(1)), ("text", string("valid"))]),
        component(&[("text", string("valid")), ("extra", NbtTag::List(vec![]))]),
        component(&[
            ("text", string("valid")),
            ("extra", NbtTag::List(vec![empty.clone()])),
        ]),
        component(&[("translate", string("key")), ("with", NbtTag::Int(1))]),
        component(&[
            ("translate", string("key")),
            ("with", NbtTag::List(vec![empty.clone()])),
        ]),
        component(&[("score", component(&[("name", string("holder"))]))]),
        component(&[("selector", string("@a")), ("separator", empty.clone())]),
        component(&[("nbt", string("path"))]),
        component(&[
            ("nbt", string("path")),
            ("storage", string("example:data")),
            ("source", string("unknown")),
        ]),
        component(&[
            ("nbt", string("path")),
            ("storage", string("example:data")),
            ("interpret", NbtTag::Byte(1)),
            ("plain", NbtTag::Byte(1)),
        ]),
        component(&[("type", string("object")), ("object", string("atlas"))]),
        component(&[("sprite", string("blocks/stone")), ("fallback", empty)]),
        component(&[("sprite", string("INVALID"))]),
        component(&[("sprite", string("..:invalid"))]),
        component(&[("player", NbtTag::Int(1))]),
    ] {
        assert!(
            !component_codec::is_valid_component(&invalid),
            "accepted malformed component {invalid:?}"
        );
    }
}

#[test]
fn component_validation_retains_opaque_types_and_vanilla_lenience() {
    for valid in [
        string(""),
        NbtTag::List(vec![string("first"), string("second")]),
        component(&[("text", string(""))]),
        // TextColor.parseColor uses Java's Unicode hexadecimal digits.
        component(&[("text", string("unicode color")), ("color", string("#١"))]),
        component(&[("type", string("text")), ("text", string("explicit"))]),
        // A successful fuzzy content codec ignores fields belonging to other codecs.
        component(&[("text", string("valid")), ("translate", NbtTag::Int(1))]),
        component(&[("keybind", string("key.jump"))]),
        component(&[("selector", string("@a")), ("separator", string(", "))]),
        component(&[(
            "score",
            component(&[("name", string("holder")), ("objective", string("points"))]),
        )]),
        component(&[
            ("translate", string("key")),
            ("with", NbtTag::IntArray(vec![1, 2])),
            ("fallback", NbtTag::Int(0)),
        ]),
        component(&[
            ("translate", string("key")),
            (
                "with",
                NbtTag::List(vec![component(&[("text", string("arg"))])]),
            ),
        ]),
        component(&[
            ("type", string("nbt")),
            ("nbt", string("path")),
            ("storage", string("example:data")),
            ("separator", component(&[])),
            ("interpret", string("ignored")),
            ("plain", NbtTag::Byte(1)),
            ("future_style", component(&[])),
        ]),
        component(&[
            ("type", string("object")),
            ("object", string("atlas")),
            ("sprite", string("blocks/stone")),
            ("future_style", NbtTag::Int(7)),
        ]),
        component(&[("player", component(&[]))]),
    ] {
        assert!(
            component_codec::is_valid_component(&valid),
            "rejected vanilla component {valid:?}"
        );
        let decoded = component_codec::read_component(&valid).unwrap();
        assert_eq!(
            component_codec::canonical_component(&component_codec::write_component(&decoded)),
            component_codec::canonical_component(&valid)
        );
    }
    // Validation is opt-in at the scoreboard boundary; legacy decoder callers keep their contract.
    assert!(component_codec::read_component(&component(&[])).is_some());
}
