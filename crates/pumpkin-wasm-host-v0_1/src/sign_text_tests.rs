use super::*;

#[test]
fn sign_plugin_strings_remain_literal_through_nbt_and_wasm_round_trip() {
    let payload =
        r#"{"text":"x","click_event":{"action":"run_command","command":"gamemode creative @s"}}"#;
    let input = SignText {
        messages: vec![payload.to_string(), "plain".to_string()],
        color: DyeColor::Red,
        has_glowing_text: true,
    };
    let text = from_wasm_sign_text(input);
    assert!(!text.has_any_click_commands(false));
    assert_eq!(text.get_message(0, false).as_ref(), payload);
    let loaded = InternalText::from(pumpkin_nbt::tag::NbtTag::from(text));
    let result = to_wasm_sign_text(&loaded);
    assert_eq!(result.messages, [payload, "plain", "", ""]);
    assert!(matches!(result.color, DyeColor::Red));
    assert!(result.has_glowing_text);
}

#[test]
fn sign_plugin_getter_returns_flattened_component_text() {
    let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
    let mut line = pumpkin_nbt::compound::NbtCompound::new();
    line.put_string("text", "first".to_string());
    line.put_list(
        "extra",
        vec![pumpkin_nbt::tag::NbtTag::String(" second".into())],
    );
    nbt.put_list(
        "messages",
        vec![pumpkin_nbt::tag::NbtTag::Compound(line); 4],
    );
    let text = InternalText::from(pumpkin_nbt::tag::NbtTag::Compound(nbt));
    assert_eq!(to_wasm_sign_text(&text).messages, ["first second"; 4]);
}
