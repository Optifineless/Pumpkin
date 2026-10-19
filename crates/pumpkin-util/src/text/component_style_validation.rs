use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use super::{
    component_codec::collection_tags,
    component_validation::{is_identifier, is_number, validate_component},
};

// Style.Serializer.MAP_CODEC uses strict optional fields; unrelated keys are ignored.
pub(super) fn is_valid_style(data: &NbtCompound, depth: usize) -> bool {
    depth <= pumpkin_nbt::MAX_NBT_DEPTH
        && [
            "bold",
            "italic",
            "underlined",
            "strikethrough",
            "obfuscated",
        ]
        .iter()
        .all(|field| optional(data, field, is_number))
        && optional(data, "color", is_string)
        && optional(data, "shadow_color", valid_shadow_color)
        && optional(data, "font", is_identifier)
        && optional(data, "insertion", is_string)
        && optional(data, "click_event", valid_click_event)
        && optional(data, "hover_event", |tag| valid_hover_event(tag, depth))
}

fn optional(data: &NbtCompound, field: &str, check: impl FnOnce(&NbtTag) -> bool) -> bool {
    data.get(field).is_none_or(check)
}

const fn is_string(tag: &NbtTag) -> bool {
    matches!(tag, NbtTag::String(_))
}

fn valid_shadow_color(tag: &NbtTag) -> bool {
    // ExtraCodecs.ARGB_COLOR_CODEC accepts an integer or a four-float vector.
    is_number(tag)
        || match tag {
            NbtTag::List(values) => values.len() == 4 && values.iter().all(is_number),
            NbtTag::ByteArray(values) => values.len() == 4,
            NbtTag::IntArray(values) => values.len() == 4,
            NbtTag::LongArray(values) => values.len() == 4,
            _ => false,
        }
}

fn valid_click_event(tag: &NbtTag) -> bool {
    let NbtTag::Compound(data) = tag else {
        return false;
    };
    // ClickEvent.Action.CODEC rejects open_file and unknown action names.
    match data.get_string("action") {
        Some("open_url") => data.get("url").is_some_and(is_string),
        Some("run_command" | "suggest_command") => {
            data.get("command").is_some_and(valid_chat_string)
        }
        Some("change_page") => data
            .get("page")
            .is_some_and(|value| numeric_int(value).is_some_and(|page| page > 0)),
        Some("copy_to_clipboard") => data.get("value").is_some_and(is_string),
        Some("custom") => data.get("id").is_some_and(is_identifier),
        Some("show_dialog") => data
            .get("dialog")
            .is_some_and(|value| is_identifier(value) || matches!(value, NbtTag::Compound(_))),
        _ => false,
    }
}

fn valid_chat_string(tag: &NbtTag) -> bool {
    matches!(tag, NbtTag::String(value) if value.chars().all(|c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}'))
}

fn numeric_int(tag: &NbtTag) -> Option<i32> {
    // Codec.INT uses Number.intValue, including narrowing from other NBT numbers.
    match tag {
        NbtTag::Byte(value) => Some(i32::from(*value)),
        NbtTag::Short(value) => Some(i32::from(*value)),
        NbtTag::Int(value) => Some(*value),
        NbtTag::Long(value) => Some(*value as i32),
        NbtTag::Float(value) => Some(*value as i32),
        NbtTag::Double(value) => Some(*value as i32),
        _ => None,
    }
}

fn valid_hover_event(tag: &NbtTag, depth: usize) -> bool {
    let NbtTag::Compound(data) = tag else {
        return false;
    };
    // HoverEvent's payload fields are alongside action, rather than inside contents.
    match data.get_string("action") {
        Some("show_text") => data
            .get("value")
            .is_some_and(|value| validate_component(value, depth + 1)),
        Some("show_entity") => {
            data.get("id").is_some_and(is_identifier)
                && data.get("uuid").is_some_and(|uuid| valid_uuid(uuid, true))
                && optional(data, "name", |value| validate_component(value, depth + 1))
        }
        Some("show_item") => {
            data.get("id").is_some_and(is_identifier)
                && optional(data, "count", |value| {
                    numeric_int(value).is_some_and(|count| (1..=99).contains(&count))
                })
                && optional(data, "components", |value| {
                    matches!(value, NbtTag::Compound(_))
                })
        }
        _ => false,
    }
}

fn valid_uuid(tag: &NbtTag, lenient: bool) -> bool {
    // UUIDUtil.CODEC is an int stream of length four; LENIENT_CODEC also permits strings.
    match tag {
        NbtTag::String(_) => lenient,
        NbtTag::List(values) => values.len() == 4 && values.iter().all(is_number),
        NbtTag::ByteArray(values) => values.len() == 4,
        NbtTag::IntArray(values) => values.len() == 4,
        NbtTag::LongArray(values) => values.len() == 4,
        _ => false,
    }
}

pub(super) fn is_valid_profile(tag: &NbtTag) -> bool {
    // ResolvableProfile.CODEC accepts a player name or a full/partial profile map.
    match tag {
        NbtTag::String(_) => valid_player_name(tag),
        NbtTag::Compound(data) => {
            optional(data, "name", valid_player_name)
                && optional(data, "id", |value| valid_uuid(value, false))
                && optional(data, "properties", valid_properties)
                && ["texture", "cape", "elytra"]
                    .iter()
                    .all(|field| optional(data, field, is_identifier))
                && optional(
                    data,
                    "model",
                    |value| matches!(value, NbtTag::String(model) if matches!(model.as_ref(), "slim" | "wide")),
                )
        }
        _ => false,
    }
}

fn valid_player_name(tag: &NbtTag) -> bool {
    // ExtraCodecs.PLAYER_NAME / StringUtil.isValidPlayerName.
    matches!(tag, NbtTag::String(name) if name.len() <= 16 && name.bytes().all(|c| c > 32 && c < 127))
}

fn valid_properties(tag: &NbtTag) -> bool {
    // ExtraCodecs.PROPERTY_MAP supports a string-to-string-list map or property records.
    match tag {
        NbtTag::Compound(data) => {
            data.child_tags.len() <= 16
                && data.child_tags.values().all(|value| {
                    collection_tags(value).is_some_and(|values| values.iter().all(is_string))
                })
        }
        NbtTag::List(values) => values.len() <= 16 && values.iter().all(valid_property),
        // NbtOps.getStream accepts all CollectionTags, including empty numeric arrays.
        NbtTag::ByteArray(values) => values.is_empty(),
        NbtTag::IntArray(values) => values.is_empty(),
        NbtTag::LongArray(values) => values.is_empty(),
        _ => false,
    }
}

fn valid_property(tag: &NbtTag) -> bool {
    let NbtTag::Compound(data) = tag else {
        return false;
    };
    data.get("name")
        .is_some_and(|value| limited_string(value, 64))
        && data
            .get("value")
            .is_some_and(|value| limited_string(value, 32767))
        && optional(data, "signature", |value| limited_string(value, 1024))
}

fn limited_string(tag: &NbtTag, max: usize) -> bool {
    matches!(tag, NbtTag::String(value) if value.encode_utf16().count() <= max)
}
