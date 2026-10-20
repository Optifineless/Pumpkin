//! Conservative display components, independent of the lossless persistence codec.

use pumpkin_nbt::{NbtCompound, tag::NbtTag};

use super::{TextComponent, color::NamedColor, component_codec};

// Leave room for packet envelopes, other component fields and NBT list wrappers.
const MAX_DISPLAY_DEPTH: usize = 128;
const MAX_DISPLAY_BYTES: usize = 32 * 1024;
// Covers compound/list headers, all supported field names, color and boolean values.
const COMPONENT_OVERHEAD: usize = 192;

struct Budget(usize);

impl Budget {
    const fn take(&mut self, bytes: usize) -> bool {
        if bytes > self.0 {
            return false;
        }
        self.0 -= bytes;
        true
    }

    fn text(&mut self, value: &str) -> String {
        let mut end = 0;
        for (offset, character) in value.char_indices() {
            // NBT uses Java's modified UTF-8, including surrogate pairs and NUL.
            let bytes = match character {
                '\0' => 2,
                _ if character.len_utf16() == 2 => 6,
                _ => character.len_utf8(),
            };
            if !self.take(bytes) {
                break;
            }
            end = offset + character.len_utf8();
        }
        value[..end].to_owned()
    }
}

/// Projects saved or plugin-provided text into the scoreboard's supported wire subset.
/// Original NBT belongs in the caller's persistence sidecar, never in this result.
#[must_use]
pub fn project(tag: &NbtTag, fallback: &str) -> TextComponent {
    let mut budget = Budget(MAX_DISPLAY_BYTES);
    let (mut projected, visible, dropped) = project_inner(tag, 1, &mut budget);
    if dropped && !visible && !fallback.is_empty() {
        for field in ["translate", "with", "fallback"] {
            projected.child_tags.remove(field);
        }
        projected.put_string("text", budget.text(fallback));
    }
    // All fields have been constructed here; the lossless decoder preserves translations
    // with fallback and their arguments without resolving them on the server.
    component_codec::read_component(&NbtTag::Compound(projected))
        .unwrap_or_else(TextComponent::empty)
}

/// Applies the same boundary to a component built during this session.
#[must_use]
pub fn component(value: &TextComponent, fallback: &str) -> TextComponent {
    project(&component_codec::write_component(value), fallback)
}

fn project_inner(tag: &NbtTag, depth: usize, budget: &mut Budget) -> (NbtCompound, bool, bool) {
    let mut result = NbtCompound::new();
    result.put_string("text", String::new());
    if depth > MAX_DISPLAY_DEPTH || !budget.take(COMPONENT_OVERHEAD) {
        return (result, false, true);
    }
    match tag {
        NbtTag::String(text) => {
            let text = budget.text(text);
            let visible = !text.is_empty();
            result.put_string("text", text);
            (result, visible, false)
        }
        NbtTag::List(children) => {
            let mut children = children.iter();
            let Some(first) = children.next() else {
                return (result, false, true);
            };
            if depth + 2 > MAX_DISPLAY_DEPTH || budget.0 < COMPONENT_OVERHEAD {
                return (result, false, true);
            }
            // Lists normalize their first child into the parent; charging two levels
            // here also bounds chains of singleton lists without growing the output.
            let (mut result, mut visible, mut dropped) = project_inner(first, depth + 2, budget);
            let mut extra = result
                .get_list("extra")
                .map(<[_]>::to_vec)
                .unwrap_or_default();
            for child in children {
                if budget.0 < COMPONENT_OVERHEAD {
                    break;
                }
                let (child, child_visible, child_dropped) = project_inner(child, depth + 2, budget);
                visible |= child_visible;
                dropped |= child_dropped;
                extra.push(NbtTag::Compound(child));
            }
            if !extra.is_empty() {
                result.put_list("extra", extra);
            }
            (result, visible, dropped)
        }
        NbtTag::Compound(data) => project_compound(data, result, depth, budget),
        _ => (result, false, true),
    }
}

fn project_compound(
    data: &NbtCompound,
    mut result: NbtCompound,
    depth: usize,
    budget: &mut Budget,
) -> (NbtCompound, bool, bool) {
    let mut visible = false;
    let mut dropped = true;
    // Honor explicit content dispatch; unsupported contents never expose their fields.
    let kind = data.get_string("type");
    if kind.is_none_or(|kind| kind == "text")
        && let Some(text) = data.get_string("text")
    {
        dropped = false;
        let text = budget.text(text);
        visible = !text.is_empty();
        result.put_string("text", text);
    } else if kind.is_none_or(|kind| kind == "translatable")
        && let Some(key) = data.get_string("translate")
    {
        dropped = false;
        result.child_tags.remove("text");
        let key = budget.text(key);
        visible = !key.is_empty();
        result.put_string("translate", key);
        if let Some(fallback) = data.get_string("fallback") {
            let fallback = budget.text(fallback);
            visible |= !fallback.is_empty();
            result.put_string("fallback", fallback);
        }
        if let Some(arguments) = data.get_list("with") {
            let children = project_children(arguments, depth, budget);
            if !children.is_empty() {
                result.put_list("with", children);
            }
        }
    } else if kind == Some("translatable") || (kind.is_none() && data.get("translate").is_some()) {
        let text = budget.text(data.get_string("fallback").unwrap_or_default());
        visible = !text.is_empty();
        result.put_string("text", text);
    }
    project_style(data, &mut result, budget);
    if let Some(children) = data.get_list("extra") {
        let mut extra = Vec::new();
        if depth + 2 <= MAX_DISPLAY_DEPTH {
            for child in children {
                if budget.0 < COMPONENT_OVERHEAD {
                    break;
                }
                let (child, child_visible, child_dropped) = project_inner(child, depth + 2, budget);
                visible |= child_visible;
                dropped |= child_dropped;
                extra.push(NbtTag::Compound(child));
            }
        }
        if !extra.is_empty() {
            result.put_list("extra", extra);
        }
    }
    (result, visible, dropped)
}

fn project_children(children: &[NbtTag], depth: usize, budget: &mut Budget) -> Vec<NbtTag> {
    let mut result = Vec::new();
    if depth + 2 <= MAX_DISPLAY_DEPTH {
        for child in children {
            if budget.0 < COMPONENT_OVERHEAD {
                break;
            }
            // Keep string arguments as strings; the conservative depth margin also
            // covers NBT's wrapper compound when a translation mixes element types.
            let child = if let NbtTag::String(text) = child {
                budget.take(COMPONENT_OVERHEAD);
                NbtTag::String(budget.text(text).into())
            } else {
                NbtTag::Compound(project_inner(child, depth + 2, budget).0)
            };
            result.push(child);
        }
    }
    result
}

fn project_style(data: &NbtCompound, result: &mut NbtCompound, budget: &mut Budget) {
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
        && budget.take(font.len())
    {
        result.put_string("font", font.to_owned());
    }
}
