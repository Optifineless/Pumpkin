use pumpkin_nbt::{NbtCompound, tag::NbtTag};

use super::{
    component_codec::collection_tags,
    component_style_validation::{is_valid_profile, is_valid_style},
};

/// Checks vanilla 26.3 component structure without changing opaque content or unknown fields.
///
/// This checks codec field shapes, not command syntax or registry-dependent payloads.
#[must_use]
pub fn is_valid_component(tag: &NbtTag) -> bool {
    validate_component(tag, 0)
}

/// Checks component structure nested in a vanilla style, including styled number formats.
#[must_use]
pub fn is_valid_component_style(style: &NbtCompound) -> bool {
    is_valid_style(style, 0)
}

pub(super) fn validate_component(tag: &NbtTag, depth: usize) -> bool {
    if depth > pumpkin_nbt::MAX_NBT_DEPTH {
        return false;
    }
    // ComponentSerialization.createCodec: string, nonempty list, or content + extra + style.
    match tag {
        NbtTag::String(_) => true,
        NbtTag::List(children) => {
            !children.is_empty()
                && children
                    .iter()
                    .all(|child| validate_component(child, depth + 1))
        }
        NbtTag::Compound(data) => {
            matches_content(data, depth)
                && data.get("extra").is_none_or(|extra| {
                    extra.extract_list().is_some_and(|children| {
                        !children.is_empty()
                            && children
                                .iter()
                                .all(|child| validate_component(child, depth + 1))
                    })
                })
                && is_valid_style(data, depth)
        }
        _ => false,
    }
}

fn matches_content(data: &NbtCompound, depth: usize) -> bool {
    // StrictEither uses a present discriminator exclusively; FuzzyCodec tries each codec.
    data.get("type").map_or_else(
        || {
            [
                "text",
                "translatable",
                "keybind",
                "score",
                "selector",
                "nbt",
                "object",
            ]
            .iter()
            .any(|kind| content(data, kind, depth))
        },
        |tag| {
            tag.extract_string()
                .is_some_and(|kind| content(data, kind, depth))
        },
    )
}

fn content(data: &NbtCompound, kind: &str, depth: usize) -> bool {
    match kind {
        "text" | "keybind" => data.get_string(kind).is_some(),
        "translatable" => {
            data.get_string("translate").is_some()
                && data.get("with").is_none_or(|arguments| {
                    collection_tags(arguments).is_some_and(|arguments| {
                        arguments.iter().all(|argument| {
                            is_number(argument) || validate_component(argument, depth + 1)
                        })
                    })
                })
            // TranslatableContents.fallback is lenientOptionalFieldOf, so invalid values are ignored.
        }
        "score" => data.get_compound("score").is_some_and(|score| {
            score.get_string("name").is_some() && score.get_string("objective").is_some()
        }),
        "selector" => {
            data.get_string("selector").is_some() && optional_component(data, "separator", depth)
        }
        "nbt" => {
            data.get_string("nbt").is_some()
                && !(numeric_bool(data.get("interpret")) && numeric_bool(data.get("plain")))
                && matches_source(data)
            // NbtContents uses lenient optional codecs for interpret, plain and separator.
        }
        "object" => matches_object(data) && optional_component(data, "fallback", depth),
        _ => false,
    }
}

fn optional_component(data: &NbtCompound, field: &str, depth: usize) -> bool {
    data.get(field)
        .is_none_or(|tag| validate_component(tag, depth + 1))
}

fn matches_source(data: &NbtCompound) -> bool {
    data.get("source").map_or_else(
        || {
            ["entity", "block", "storage"]
                .iter()
                .any(|kind| source(data, kind))
        },
        |tag| tag.extract_string().is_some_and(|kind| source(data, kind)),
    )
}

fn source(data: &NbtCompound, kind: &str) -> bool {
    match kind {
        "entity" | "block" => data.get_string(kind).is_some(),
        "storage" => data.get("storage").is_some_and(is_identifier),
        _ => false,
    }
}

fn matches_object(data: &NbtCompound) -> bool {
    // ObjectInfos uses `object`, not `type`; atlas/player are selected by sprite/player keys.
    data.get("object").map_or_else(
        || ["atlas", "player"].iter().any(|kind| object(data, kind)),
        |tag| tag.extract_string().is_some_and(|kind| object(data, kind)),
    )
}

fn object(data: &NbtCompound, kind: &str) -> bool {
    match kind {
        "atlas" => {
            data.get("sprite").is_some_and(is_identifier)
                && data.get("atlas").is_none_or(is_identifier)
        }
        "player" => {
            data.get("player").is_some_and(is_valid_profile)
                && data.get("hat").is_none_or(is_number)
        }
        _ => false,
    }
}

pub(super) const fn is_number(tag: &NbtTag) -> bool {
    matches!(
        tag,
        NbtTag::Byte(_)
            | NbtTag::Short(_)
            | NbtTag::Int(_)
            | NbtTag::Long(_)
            | NbtTag::Float(_)
            | NbtTag::Double(_)
    )
}

pub(super) fn is_identifier(tag: &NbtTag) -> bool {
    tag.extract_string().is_some_and(|value| {
        // Identifier.isValidNamespace also excludes the parent-directory namespace.
        value
            .split_once(':')
            .is_none_or(|(namespace, _)| namespace != "..")
            && crate::identifier::Identifier::parse(value).is_ok()
    })
}

fn numeric_bool(tag: Option<&NbtTag>) -> bool {
    // NbtOps.getBooleanValue tests Number.doubleValue != 0, including non-byte tags.
    match tag {
        Some(NbtTag::Byte(value)) => *value != 0,
        Some(NbtTag::Short(value)) => *value != 0,
        Some(NbtTag::Int(value)) => *value != 0,
        Some(NbtTag::Long(value)) => *value != 0,
        Some(NbtTag::Float(value)) => *value != 0.0,
        Some(NbtTag::Double(value)) => *value != 0.0,
        _ => false,
    }
}
