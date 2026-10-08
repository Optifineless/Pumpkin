use super::TextComponent;
use pumpkin_nbt::tag::NbtTag;
use serde_json::Value;

// AtlasSprite.DEFAULT_ATLAS is AtlasIds.BLOCKS.
const DEFAULT_ATLAS: &str = "minecraft:blocks";

/// Encodes a component without translating or losing its NBT contents.
#[must_use]
pub fn write_component(component: &TextComponent) -> NbtTag {
    component
        .0
        .to_nbt_tag_for_version(&crate::version::JavaMinecraftVersion::V_26_3)
}

/// Decodes component NBT, retaining contents the structured text model cannot express.
#[must_use]
pub fn read_component(tag: &NbtTag) -> Option<TextComponent> {
    let supported = read_supported_component(tag);
    if let NbtTag::Compound(compound) = tag
        && supported.as_ref().is_none_or(|value| {
            canonical_component(&write_component(value)) != canonical_component(tag)
        })
    {
        // ComponentSerialization.bootstrap: retain contents/styles the renderer cannot model.
        let component = super::ProfileNbt(compound.clone());
        let content = if compound.get_string("translate").is_some() {
            super::TextContent::Translatable { component }
        } else {
            super::TextContent::Opaque { component }
        };
        return Some(TextComponent::from_content(content));
    }
    supported
}

fn read_supported_component(tag: &NbtTag) -> Option<TextComponent> {
    // ComponentSerialization.CODEC/createFromList preserves the first component's style.
    match tag {
        NbtTag::String(text) => Some(TextComponent::text(text.to_string())),
        NbtTag::List(tags) => {
            let mut tags = tags.iter();
            let mut result = read_component(tags.next()?)?;
            for tag in tags {
                result.0.extra.push(read_component(tag)?.0);
            }
            Some(result)
        }
        NbtTag::Compound(compound) => {
            let mut value = nbt_json(tag);
            for field in ["extra", "with"] {
                if let Some(children) = compound.get(field) {
                    let children = children.extract_list()?;
                    if field == "extra" && children.is_empty() {
                        return None;
                    }
                    value[field] =
                        Value::Array(children.iter().map(component_json).collect::<Option<_>>()?);
                }
            }
            if let Some(hover) = compound.get_compound("hover_event") {
                let field = match hover.get_string("action")? {
                    "show_text" => Some("value"),
                    "show_entity" => Some("name"),
                    _ => None,
                };
                if let Some(field) = field
                    && let Some(component) = hover.get(field)
                {
                    let children = if let NbtTag::List(children) = component {
                        children
                            .iter()
                            .map(component_json)
                            .collect::<Option<Vec<_>>>()?
                    } else {
                        vec![component_json(component)?]
                    };
                    value["hover_event"][field] = Value::Array(children);
                }
            }
            serde_json::from_value(value).ok()
        }
        _ => None,
    }
}

/// Produces the decoded component value used by custom-name equality and hashing.
#[must_use]
pub fn canonical_component(tag: &NbtTag) -> Value {
    let mut value = nbt_json(tag);
    normalize_component(&mut value);
    value
}

fn normalize_component(value: &mut Value) {
    if let Value::String(text) = value {
        *value = serde_json::json!({"text":text});
    }
    if let Value::Array(children) = value {
        let mut children = std::mem::take(children).into_iter();
        if let Some(mut first) = children.next() {
            normalize_component(&mut first);
            let extra = first["extra"].as_array().cloned().unwrap_or_default();
            let mut remaining = extra;
            remaining.extend(children.map(|mut child| {
                normalize_component(&mut child);
                child
            }));
            if !remaining.is_empty() {
                first["extra"] = Value::Array(remaining);
            }
            *value = first;
        }
    }
    let Value::Object(fields) = value else {
        return;
    };
    normalize_style(fields);
    normalize_contents(fields);
    normalize_children(fields);
}

fn normalize_style(fields: &mut serde_json::Map<String, Value>) {
    // TextColor.equals compares RGB; Style.equals compares canonical identifiers.
    if let Some(Value::String(color)) = fields.get_mut("color") {
        if let Ok(named) = super::color::NamedColor::try_from(color.as_str()) {
            let rgb = named.to_rgb();
            *color = format!("#{:02x}{:02x}{:02x}", rgb.red, rgb.green, rgb.blue);
        } else if let Some(hex) = color.strip_prefix('#')
            && let Ok(rgb) = i32::from_str_radix(hex, 16)
            && (0..=0x00ff_ffff).contains(&rgb)
        {
            // TextColor.parseColor accepts any hexadecimal spelling in the RGB range.
            *color = format!("#{rgb:06x}");
        }
    }
    for field in ["font", "atlas", "sprite", "storage"] {
        if let Some(Value::String(id)) = fields.get_mut(field)
            && !id.contains(':')
        {
            *id = format!("minecraft:{id}");
        }
    }
    if let Some(Value::Object(hover)) = fields.get_mut("hover_event") {
        for field in ["value", "name"] {
            if let Some(child) = hover.get_mut(field) {
                normalize_component(child);
            }
        }
    }
}

fn normalize_contents(fields: &mut serde_json::Map<String, Value>) {
    if fields
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| {
            matches!(
                kind,
                "text" | "translatable" | "keybind" | "score" | "selector" | "nbt" | "object"
            )
        })
    {
        fields.remove("type");
    }
    // NbtContents and AtlasSprite codec defaults participate in decoded value equality.
    if fields.contains_key("nbt") {
        fields.entry("interpret").or_insert(Value::Bool(false));
    }
    if fields.contains_key("sprite") {
        fields
            .entry("atlas")
            .or_insert(Value::String(DEFAULT_ATLAS.into()));
    }
    if let Some(player) = fields.get_mut("player") {
        // ResolvableProfile.CODEC's name alternative creates the same unresolved profile.
        if let Value::String(name) = player {
            *player = serde_json::json!({"name": name});
        }
        // PlayerSprite.MAP_CODEC defaults hat to true.
        fields.entry("hat").or_insert(Value::Bool(true));
    }
    if fields.contains_key("sprite") || fields.contains_key("player") {
        fields.remove("object");
        if let Some(fallback) = fields.get_mut("fallback") {
            normalize_component(fallback);
        }
    }
    if let Some(separator) = fields.get_mut("separator") {
        normalize_component(separator);
    }
}

fn normalize_children(fields: &mut serde_json::Map<String, Value>) {
    for field in ["extra", "with"] {
        if let Some(Value::Array(children)) = fields.get_mut(field) {
            for child in children {
                if field == "extra" || child.is_object() || child.is_array() {
                    normalize_component(child);
                }
                if field == "with"
                    && child
                        .as_object()
                        .is_some_and(|c| c.len() == 1 && c.contains_key("text"))
                {
                    *child = child["text"].clone();
                }
            }
        }
    }
    if fields
        .get("with")
        .is_some_and(|args| args.as_array().is_some_and(Vec::is_empty))
    {
        fields.remove("with");
    }
}

fn component_json(tag: &NbtTag) -> Option<Value> {
    serde_json::to_value(read_component(tag)?.0).ok()
}

fn nbt_json(tag: &NbtTag) -> Value {
    // Style.Serializer.MAP_CODEC uses boolean NBT bytes; other bytes remain numeric.
    match tag {
        NbtTag::Compound(compound) => Value::Object(
            compound
                .child_tags
                .iter()
                .map(|(name, tag)| {
                    let value = if matches!(
                        name.as_ref(),
                        "bold"
                            | "italic"
                            | "underlined"
                            | "strikethrough"
                            | "obfuscated"
                            | "interpret"
                            | "plain"
                            | "hat"
                    ) && let NbtTag::Byte(value) = tag
                    {
                        Value::Bool(*value != 0)
                    } else if name.as_ref() == "with"
                        && let Some(arguments) = collection_tags(tag)
                    {
                        Value::Array(arguments.iter().map(argument_json).collect())
                    } else if name.as_ref() == "shadow_color" {
                        shadow_color_json(tag)
                    } else {
                        nbt_json(tag)
                    };
                    (name.to_string(), value)
                })
                .collect(),
        ),
        NbtTag::List(tags) => Value::Array(tags.iter().map(nbt_json).collect()),
        NbtTag::End => Value::Null,
        NbtTag::Byte(value) => (*value).into(),
        NbtTag::Short(value) => (*value).into(),
        NbtTag::Int(value) => (*value).into(),
        NbtTag::Long(value) => (*value).into(),
        NbtTag::Float(value) => (*value).into(),
        NbtTag::Double(value) => (*value).into(),
        NbtTag::String(value) => value.to_string().into(),
        NbtTag::ByteArray(values) => Value::Array(values.iter().map(|v| Value::from(*v)).collect()),
        NbtTag::IntArray(values) => Value::Array(values.iter().map(|v| Value::from(*v)).collect()),
        NbtTag::LongArray(values) => Value::Array(values.iter().map(|v| Value::from(*v)).collect()),
    }
}

fn shadow_color_json(tag: &NbtTag) -> Value {
    use super::color::ARGBColor;
    const CHANNEL_SCALE: f32 = 255.0;
    let packed = match tag {
        NbtTag::Byte(value) => Some(i32::from(*value)),
        NbtTag::Short(value) => Some(i32::from(*value)),
        NbtTag::Int(value) => Some(*value),
        NbtTag::Long(value) => Some(*value as i32),
        NbtTag::Float(value) => Some(*value as i32),
        NbtTag::Double(value) => Some(*value as i32),
        _ => None,
    };
    let color = if let Some(packed) = packed {
        ARGBColor::from_argb_int(packed)
    } else {
        let value = nbt_json(tag);
        let Some(values) = value.as_array() else {
            return value;
        };
        let [red, green, blue, alpha] = values.as_slice() else {
            return value;
        };
        let [Some(red), Some(green), Some(blue), Some(alpha)] =
            [red, green, blue, alpha].map(Value::as_f64)
        else {
            return value;
        };
        // ExtraCodecs.ARGB_COLOR_CODEC -> ARGB.colorFromFloat / as8BitChannel.
        let channel = |value: f64| (value as f32 * CHANNEL_SCALE).floor() as i32 as u8;
        ARGBColor::new(channel(alpha), channel(red), channel(green), channel(blue))
    };
    serde_json::json!({
        "alpha": color.alpha,
        "red": color.red,
        "green": color.green,
        "blue": color.blue,
    })
}

fn argument_json(tag: &NbtTag) -> Value {
    match tag {
        NbtTag::Byte(_)
        | NbtTag::Short(_)
        | NbtTag::Int(_)
        | NbtTag::Long(_)
        | NbtTag::Float(_)
        | NbtTag::Double(_) => {
            // NbtOps.convertTo -> ExtraCodecs.JAVA retains boxed number types; Arrays.equals does too.
            serde_json::json!({"$primitive_number": format!("{tag:?}")})
        }
        _ => nbt_json(tag),
    }
}

// NbtOps.getList accepts ListTag and all three primitive array tags.
pub(super) fn collection_tags(tag: &NbtTag) -> Option<std::borrow::Cow<'_, [NbtTag]>> {
    use std::borrow::Cow;
    match tag {
        NbtTag::List(tags) => Some(Cow::Borrowed(tags)),
        NbtTag::ByteArray(values) => Some(Cow::Owned(
            values.iter().copied().map(NbtTag::Byte).collect(),
        )),
        NbtTag::IntArray(values) => Some(Cow::Owned(
            values.iter().copied().map(NbtTag::Int).collect(),
        )),
        NbtTag::LongArray(values) => Some(Cow::Owned(
            values.iter().copied().map(NbtTag::Long).collect(),
        )),
        _ => None,
    }
}
