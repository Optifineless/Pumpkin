use super::{TextComponent, translatable};
use crate::translation::Locale;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};

fn repeated(depth: usize, leaf: &str) -> NbtCompound {
    let mut value = NbtTag::String(leaf.into());
    for _ in 0..depth {
        let mut component = NbtCompound::new();
        component.put_string("translate", "review.nonexistent".into());
        component.put_string("fallback", "%1$s%1$s".into());
        component.put_list("with", vec![value]);
        value = NbtTag::Compound(component);
    }
    value.extract_compound().unwrap().clone()
}

#[test]
fn translated_text_bounds_repeated_arguments_before_allocation() {
    // T16 is a safe negative control: the old renderer expands to only 64 KiB.
    let text = translatable::get_text(&repeated(16, "A"), Locale::EnUs);
    assert!(!text.is_empty());
    assert!(text.len() <= 32 * 1024, "Expanded to {} bytes", text.len());
}

#[test]
fn translated_text_bounds_nested_empty_arguments_and_utf8() {
    for leaf in ["", "😀"] {
        let text = translatable::get_text(&repeated(40, leaf), Locale::EnUs);
        assert!(text.len() <= 32 * 1024);
        assert!(text.chars().all(|c| c == '😀'));
    }
}

#[test]
fn translated_text_keeps_template_errors_arguments_and_extras() {
    let mut component = repeated(1, "Alice");
    component.put_list("extra", vec![NbtTag::String("!".into())]);
    assert_eq!(
        TextComponent::from_nbt(&NbtTag::Compound(component.clone())).get_text(),
        "AliceAlice!"
    );
    for template in ["%2$s", "%1$d", "prefix %s %s"] {
        component.put_string("fallback", template.into());
        assert_eq!(
            translatable::get_text(&component, Locale::EnUs),
            format!("{template}!")
        );
    }
    component.put_string("fallback", "%% %1$s".into());
    assert_eq!(translatable::get_text(&component, Locale::EnUs), "% Alice!");
}

#[test]
fn translated_text_keeps_other_component_arguments() {
    let mut component = repeated(1, "");
    component.put_string("fallback", "%s".into());
    for (field, value) in [("keybind", "key.jump"), ("selector", "@a")] {
        let mut argument = NbtCompound::new();
        argument.put_string(field, value.into());
        component.put_list("with", vec![NbtTag::Compound(argument)]);
        assert_eq!(translatable::get_text(&component, Locale::EnUs), value);
    }
}

#[test]
fn translated_text_shares_budget_across_literal_and_structured_roots() {
    let children = vec![NbtTag::Compound(repeated(40, "AAAA")); 2];
    let mut root = NbtCompound::new();
    root.put_string("text", String::new());
    root.put_list("extra", children.clone());
    let text = TextComponent::from_nbt(&NbtTag::Compound(root)).get_text();
    assert!(!text.is_empty());
    assert!(text.len() <= 32 * 1024, "Expanded to {} bytes", text.len());
    let mut component = TextComponent::text("");
    component.0.extra = children
        .iter()
        .map(|child| TextComponent::from_nbt(child).0)
        .collect();
    assert!(component.get_text().len() <= 32 * 1024);
}

#[test]
fn translated_text_keeps_native_placeholder_rules_with_shared_budget() {
    use super::TextContent;
    crate::translation::add_translation(
        "render_review",
        "arguments",
        "%2$d %% %s \\%s %99$s",
        Locale::EnUs,
    );
    let component = TextComponent::from_content(TextContent::Custom {
        key: "render_review:arguments".into(),
        locale: Locale::EnUs,
        with: vec![TextComponent::text("A").0, TextComponent::text("B").0],
    });
    assert_eq!(component.get_text(), "B % A \\%s B");
}
