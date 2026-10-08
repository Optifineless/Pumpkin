use super::DataComponentImpl;
use super::basic::CustomNameImpl;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::text::TextComponent;

#[test]
fn review3_known_translation_precedes_fallback() {
    let mut nbt = NbtCompound::new();
    nbt.put_string("translate", "container.chest".into());
    nbt.put_string("fallback", "Vault".into());
    let tag = NbtTag::Compound(nbt);
    let name = CustomNameImpl::read_data(&tag).unwrap();
    assert_eq!(name.name.clone().get_text(), "Chest");
    assert_eq!(name.write_data(), tag);
}

#[test]
fn review3_numeric_translation_substitutes_arguments() {
    let mut nbt = NbtCompound::new();
    nbt.put_string("translate", "unknown.key".into());
    nbt.put_string("fallback", "Key %2$s / %s / %%".into());
    nbt.put_list("with", vec![NbtTag::Int(1), NbtTag::Long(2)]);
    let tag = NbtTag::Compound(nbt);
    let name = CustomNameImpl::read_data(&tag).unwrap();
    assert_eq!(name.name.clone().get_text(), "Key 2 / 1 / %");
    assert_eq!(name.write_data(), tag);
    for (argument, expected) in [
        (NbtTag::Float(1.0), "Value 1.0"),
        (NbtTag::Double(-0.0), "Value -0.0"),
        (NbtTag::Double(1e7), "Value 1.0E7"),
        (NbtTag::Double(0.001), "Value 0.001"),
        (NbtTag::Double(0.0001), "Value 1.0E-4"),
    ] {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", "unknown.key".into());
        nbt.put_string("fallback", "Value %s".into());
        nbt.put_list("with", vec![argument]);
        let tag = NbtTag::Compound(nbt);
        let name = CustomNameImpl::read_data(&tag).unwrap();
        assert_eq!(name.name.clone().get_text(), expected);
        assert_eq!(name.write_data(), tag);
    }
}

#[test]
fn review3_lock_colors_compare_and_hash_decoded_rgb() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let name = |color: &str| {
        let mut nbt = NbtCompound::new();
        nbt.put_string("text", "Key".into());
        nbt.put_string("color", color.into());
        CustomNameImpl::read_data(&NbtTag::Compound(nbt)).unwrap()
    };
    let hash = |name: &CustomNameImpl| {
        let mut state = DefaultHasher::new();
        name.hash(&mut state);
        state.finish()
    };
    for color in ["#0", "#000000", "#00000000", "#+0", "#-0"] {
        assert_eq!(name("black"), name(color), "{color}");
        assert_eq!(hash(&name("black")), hash(&name(color)), "{color}");
    }
    assert_ne!(name("black"), name("#1"));
}

#[test]
fn review3_lock_argument_collections_keep_boxed_number_types() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let name = |arguments| {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", "unknown.key".into());
        nbt.put("with", arguments);
        CustomNameImpl::read_data(&NbtTag::Compound(nbt)).unwrap()
    };
    let hash = |name: &CustomNameImpl| {
        let mut state = DefaultHasher::new();
        name.hash(&mut state);
        state.finish()
    };
    for (array, argument) in [
        (NbtTag::ByteArray(vec![1].into()), NbtTag::Byte(1)),
        (NbtTag::IntArray(vec![1]), NbtTag::Int(1)),
        (NbtTag::LongArray(vec![1]), NbtTag::Long(1)),
    ] {
        let array = name(array);
        let list = name(NbtTag::List(vec![argument]));
        assert_eq!(array, list);
        assert_eq!(hash(&array), hash(&list));
    }
}

#[test]
fn review3_lock_integer_and_long_arrays_are_distinct() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let name = |arguments| {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", "unknown.key".into());
        nbt.put("with", arguments);
        CustomNameImpl::read_data(&NbtTag::Compound(nbt)).unwrap()
    };
    let hash = |name: &CustomNameImpl| {
        let mut state = DefaultHasher::new();
        name.hash(&mut state);
        state.finish()
    };
    let integer = name(NbtTag::IntArray(vec![1]));
    let long = name(NbtTag::LongArray(vec![1]));
    assert_ne!(integer, long);
    assert_ne!(hash(&integer), hash(&long));
}

#[test]
fn opaque_list_component_keeps_existing_and_appended_children() {
    let mut score = NbtCompound::new();
    score.put_string("name", "Player".into());
    score.put_string("objective", "points".into());
    let mut first = NbtCompound::new();
    first.put_compound("score", score);
    first.put_list("extra", vec![NbtTag::String("existing".into())]);
    let list = NbtTag::List(vec![
        NbtTag::Compound(first),
        NbtTag::String("appended".into()),
    ]);
    let component = CustomNameImpl::read_data(&list).unwrap();
    assert_eq!(
        component
            .write_data()
            .extract_compound()
            .unwrap()
            .get_list("extra")
            .unwrap(),
        &[
            NbtTag::String("existing".into()),
            NbtTag::String("appended".into())
        ]
    );
}

#[test]
fn vanilla_name_fixtures_decode_complete_item_stacks_and_preserve_contents() {
    for bytes in [
        include_bytes!("../../tests/fixtures/custom_names/translation.nbt").as_slice(),
        include_bytes!("../../tests/fixtures/custom_names/score.nbt").as_slice(),
        include_bytes!("../../tests/fixtures/custom_names/nbt.nbt").as_slice(),
        include_bytes!("../../tests/fixtures/custom_names/object.nbt").as_slice(),
        include_bytes!("../../tests/fixtures/custom_names/future.nbt").as_slice(),
    ] {
        let fixture =
            pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
                &mut std::io::Cursor::new(bytes),
            ))
            .unwrap();
        let item = crate::item_stack::ItemStack::read_item_stack(&fixture.root_tag).unwrap();
        let mut saved = NbtCompound::new();
        item.write_item_stack(&mut saved);
        assert_eq!(
            saved.get_compound("components"),
            fixture.root_tag.get_compound("components")
        );
        assert!(crate::item_stack::ItemStack::read_item_stack(&saved).is_some());
    }
}

#[test]
fn component_equality_compares_rgb_identifiers_fallback_style_and_children() {
    fn component(
        color: &str,
        font: &str,
        fallback: &str,
        bold: bool,
        child: &str,
    ) -> CustomNameImpl {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", "example".into());
        nbt.put_string("fallback", fallback.into());
        nbt.put_string("font", font.into());
        nbt.put_string("color", color.into());
        nbt.put_bool("bold", bold);
        nbt.put_list("extra", vec![NbtTag::String(child.into())]);
        CustomNameImpl::read_data(&NbtTag::Compound(nbt)).unwrap()
    }
    let name = component("red", "default", "Key", true, "!");
    assert_eq!(
        name,
        component("#FF5555", "minecraft:default", "Key", true, "!")
    );
    assert_ne!(name, component("red", "default", "Other", true, "!"));
    assert_ne!(name, component("red", "default", "Key", false, "!"));
    assert_ne!(name, component("red", "default", "Key", true, "?"));
    let mut sprite = NbtCompound::new();
    sprite.put_string("player", "Player".into());
    let implicit = CustomNameImpl::read_data(&NbtTag::Compound(sprite.clone())).unwrap();
    let mut profile = NbtCompound::new();
    profile.put_string("name", "Player".into());
    sprite.put_compound("player", profile);
    sprite.put_bool("hat", true);
    assert_eq!(
        implicit,
        CustomNameImpl::read_data(&NbtTag::Compound(sprite.clone())).unwrap()
    );
    sprite.put_bool("hat", false);
    assert_ne!(
        implicit,
        CustomNameImpl::read_data(&NbtTag::Compound(sprite)).unwrap()
    );
}

#[test]
fn component_equality_normalizes_shadow_color_vectors() {
    fn name(shadow: NbtTag) -> CustomNameImpl {
        let mut nbt = NbtCompound::new();
        nbt.put_string("text", "Key".into());
        nbt.put("shadow_color", shadow);
        CustomNameImpl::read_data(&NbtTag::Compound(nbt)).unwrap()
    }
    // ExtraCodecs.ARGB_COLOR_CODEC / ARGB.colorFromFloat use RGBA order and floor channels.
    let vector = |values: [f32; 4]| NbtTag::List(values.map(NbtTag::Float).to_vec());
    assert_eq!(
        name(NbtTag::Int(-65536)),
        name(vector([1.0, 0.0, 0.0, 1.0]))
    );
    assert_eq!(
        name(NbtTag::Int(0x7f7f_3fbf)),
        name(vector([0.5, 0.25, 0.75, 0.5]))
    );
    assert_ne!(
        name(NbtTag::Int(-65536)),
        name(vector([1.0, 0.0, 0.0, 0.5]))
    );
}

#[test]
fn component_equality_retains_primitive_argument_types() {
    fn name(argument: NbtTag) -> CustomNameImpl {
        let mut value = NbtCompound::new();
        value.put_string("translate", "example".into());
        value.put_list("with", vec![argument]);
        CustomNameImpl::read_data(&NbtTag::Compound(value)).unwrap()
    }
    assert_ne!(name(NbtTag::Short(1)), name(NbtTag::Int(1)));
    assert_ne!(name(NbtTag::Int(1)), name(NbtTag::Double(1.0)));
    assert_ne!(name(NbtTag::Double(0.0)), name(NbtTag::Double(-0.0)));
}

#[test]
fn custom_name_round_trips_styles_children_and_translation() {
    for name in [
        TextComponent::text("Vault key").bold().italic(),
        TextComponent::text("Shadow key")
            .shadow_color(pumpkin_util::text::color::ARGBColor::new(128, 12, 34, 56))
            .add_child(TextComponent::text(" nested").shadow_color(
                pumpkin_util::text::color::ARGBColor::new(255, 65, 43, 21),
            )),
        serde_json::from_value(serde_json::json!({"translate": "container.chest", "with": [{"text": "key", "bold": true}]})).unwrap(),
        TextComponent::text("Vault").bold().add_child(TextComponent::text(" key").italic()),
        TextComponent::text("Vault").add_child(TextComponent::text(" plain")).add_child(TextComponent::text(" styled").bold()),
        serde_json::from_value(serde_json::json!({"text":"Vault", "italic":false, "color":"red", "hover_event":{"action":"show_text", "value":[{"text":"hint"}]}, "click_event":{"action":"run_command","command":"/help"}})).unwrap(),
        serde_json::from_value(serde_json::json!({"translate":"container.chest", "with":[{"text":"plain"}]})).unwrap(),
    ] {
        let component = CustomNameImpl { name };
        let mut root = NbtCompound::new();
        root.put("name", component.write_data());
        let bytes = pumpkin_nbt::Nbt::new(String::new(), root).write();
        let mut cursor = std::io::Cursor::new(bytes.as_ref());
        let decoded = pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor)).unwrap();
        assert_eq!(CustomNameImpl::read_data(decoded.root_tag.get("name").unwrap()), Some(component));
    }
}

#[test]
fn custom_name_accepts_string_compound_and_nonempty_list() {
    let mut styled = NbtCompound::new();
    styled.put_string("text", "Vault".to_owned());
    styled.put_bool("bold", true);
    let styled = NbtTag::Compound(styled);
    assert_eq!(
        CustomNameImpl::read_data(&styled).unwrap().name,
        TextComponent::text("Vault").bold()
    );
    let list = NbtTag::List(vec![styled, NbtTag::String(" key".into())]);
    let decoded = CustomNameImpl::read_data(&list).unwrap();
    assert_eq!(
        decoded.name,
        TextComponent::text("Vault")
            .bold()
            .add_child(TextComponent::text(" key"))
    );
    assert_eq!(
        CustomNameImpl::read_data(&decoded.write_data()),
        Some(decoded)
    );
    assert!(CustomNameImpl::read_data(&NbtTag::List(Vec::new())).is_none());
    assert_eq!(
        CustomNameImpl::read_data(&NbtTag::String("literal".into()))
            .unwrap()
            .name,
        TextComponent::text("literal")
    );
}
