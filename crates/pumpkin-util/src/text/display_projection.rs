//! Conservative display components, independent of the lossless persistence codec.

use pumpkin_nbt::{NbtCompound, tag::NbtTag};

use super::{TextComponent, color::NamedColor, component_codec};

/// Projects saved or plugin-provided text into the scoreboard's supported wire subset.
/// Original NBT belongs in the caller's persistence sidecar, never in this result.
#[must_use]
pub fn project(tag: &NbtTag, fallback: &str) -> TextComponent {
    let (mut projected, visible, dropped) = project_inner(tag, 0);
    if dropped && !visible && !fallback.is_empty() {
        for field in ["translate", "with", "fallback"] {
            projected.child_tags.remove(field);
        }
        projected.put_string("text", fallback.to_owned());
    }
    // All fields have been constructed here; the lossless decoder preserves translations
    // with fallback and their arguments without resolving them on the server.
    component_codec::read_component(&NbtTag::Compound(projected))
        .unwrap_or_else(|| TextComponent::text(fallback.to_owned()))
}

/// Applies the same boundary to a component built during this session.
#[must_use]
pub fn component(value: &TextComponent, fallback: &str) -> TextComponent {
    project(&component_codec::write_component(value), fallback)
}

fn project_inner(tag: &NbtTag, depth: usize) -> (NbtCompound, bool, bool) {
    let mut result = NbtCompound::new();
    result.put_string("text", String::new());
    if depth > pumpkin_nbt::MAX_NBT_DEPTH {
        return (result, false, true);
    }
    let mut visible = false;
    let mut dropped = true;
    match tag {
        NbtTag::String(text) => {
            dropped = false;
            visible = !text.is_empty();
            result.put_string("text", text.to_string());
        }
        NbtTag::List(children) => {
            let mut children = children.iter();
            if let Some(first) = children.next() {
                (result, visible, dropped) = project_inner(first, depth + 1);
                let mut extra = result
                    .get_list("extra")
                    .map(<[_]>::to_vec)
                    .unwrap_or_default();
                for child in children {
                    let (child, child_visible, child_dropped) = project_inner(child, depth + 1);
                    visible |= child_visible;
                    dropped |= child_dropped;
                    extra.push(NbtTag::Compound(child));
                }
                if !extra.is_empty() {
                    result.put_list("extra", extra);
                }
            }
        }
        NbtTag::Compound(data) => {
            // Honor explicit content dispatch; unsupported contents never expose their fields.
            let kind = data.get_string("type");
            if kind.is_none_or(|kind| kind == "text")
                && let Some(text) = data.get_string("text")
            {
                dropped = false;
                visible = !text.is_empty();
                result.put_string("text", text.to_owned());
            } else if kind.is_none_or(|kind| kind == "translatable")
                && let Some(key) = data.get_string("translate")
            {
                dropped = false;
                result.child_tags.remove("text");
                result.put_string("translate", key.to_owned());
                visible = !key.is_empty();
                if let Some(fallback) = data.get_string("fallback") {
                    visible |= !fallback.is_empty();
                    result.put_string("fallback", fallback.to_owned());
                }
                if let Some(arguments) = data.get_list("with")
                    && !arguments.is_empty()
                {
                    result.put_list(
                        "with",
                        arguments
                            .iter()
                            .map(|argument| {
                                if let NbtTag::String(_) = argument {
                                    argument.clone()
                                } else {
                                    NbtTag::Compound(project_inner(argument, depth + 1).0)
                                }
                            })
                            .collect(),
                    );
                }
            } else if kind == Some("translatable")
                || (kind.is_none() && data.get("translate").is_some())
            {
                let text = data.get_string("fallback").unwrap_or_default();
                visible = !text.is_empty();
                result.put_string("text", text.to_owned());
            }
            project_style(data, &mut result);
            if let Some(children) = data.get_list("extra")
                && !children.is_empty()
            {
                let extra = children
                    .iter()
                    .map(|child| {
                        let (child, child_visible, child_dropped) = project_inner(child, depth + 1);
                        visible |= child_visible;
                        dropped |= child_dropped;
                        NbtTag::Compound(child)
                    })
                    .collect();
                result.put_list("extra", extra);
            }
        }
        _ => {}
    }
    (result, visible, dropped)
}

fn project_style(data: &NbtCompound, result: &mut NbtCompound) {
    if let Some(color) = data.get_string("color")
        && (NamedColor::try_from(color).is_ok()
            || color.strip_prefix('#').is_some_and(|hex| {
                hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
            }))
    {
        result.put_string("color", color.to_owned());
    }
    for field in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        if let Some(value) = data.get_bool(field) {
            result.put_bool(field, value);
        }
    }
    if let Some(font) = data.get_string("font")
        && font
            .split_once(':')
            .is_none_or(|(namespace, _)| namespace != "..")
        && crate::identifier::Identifier::parse(font).is_ok()
    {
        result.put_string("font", font.to_owned());
    }
}
