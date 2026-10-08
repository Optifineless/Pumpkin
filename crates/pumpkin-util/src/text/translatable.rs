use super::{TextComponent, component_codec::collection_tags};
use crate::translation::{Locale, get_translation_with_fallback};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};

pub(super) fn get_text(component: &NbtCompound, locale: Locale) -> String {
    let key = component.get_string("translate").unwrap_or_default();
    // TranslatableContents.decompose selects Language.getOrDefault(key, fallback) first.
    let template = get_translation_with_fallback(
        &format!("minecraft:{key}"),
        locale,
        component.get_string("fallback").unwrap_or(key),
    );
    let arguments = component
        .get("with")
        .and_then(collection_tags)
        .map(|arguments| {
            arguments
                .iter()
                .map(|argument| argument_text(argument, locale))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut text = decompose_template(&template, &arguments).unwrap_or(template);
    if let Some(children) = component.get_list("extra") {
        for child in children {
            text.push_str(&TextComponent::from_nbt(child).0.get_text(locale));
        }
    }
    text
}

fn argument_text(argument: &NbtTag, locale: Locale) -> String {
    // TranslatableContents.getArgument uses Number.toString or visits a component.
    match argument {
        NbtTag::Byte(value) => value.to_string(),
        NbtTag::Short(value) => value.to_string(),
        NbtTag::Int(value) => value.to_string(),
        NbtTag::Long(value) => value.to_string(),
        NbtTag::Float(value) => floating_point_text(format!("{value:?}")),
        NbtTag::Double(value) => floating_point_text(format!("{value:?}")),
        NbtTag::String(value) => value.to_string(),
        _ => TextComponent::from_nbt(argument).0.get_text(locale),
    }
}

fn floating_point_text(value: String) -> String {
    if value == "inf" || value == "-inf" {
        return value.replace("inf", "Infinity");
    }
    let (sign, unsigned) = value
        .strip_prefix('-')
        .map_or(("", value.as_str()), |unsigned| ("-", unsigned));
    let (mantissa, exponent) = unsigned.split_once('e').unwrap_or((unsigned, "0"));
    let Ok(exponent) = exponent.parse::<i32>() else {
        return value;
    };
    let decimal = mantissa.find('.').unwrap_or(mantissa.len());
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let Some(first) = digits.find(|c| c != '0') else {
        return format!("{sign}0.0");
    };
    if !digits.bytes().all(|c| c.is_ascii_digit()) {
        return value;
    }
    let exponent = exponent + decimal as i32 - first as i32 - 1;
    let digits = digits[first..].trim_end_matches('0');
    // Float/Double.toString use decimal notation for exponents -3 through 6.
    if (-3..7).contains(&exponent) {
        let decimal = exponent + 1;
        if decimal <= 0 {
            return format!("{sign}0.{}{digits}", "0".repeat((-decimal) as usize));
        }
        let decimal = decimal as usize;
        if decimal >= digits.len() {
            return format!("{sign}{digits}{}.0", "0".repeat(decimal - digits.len()));
        }
        return format!("{sign}{}.{}", &digits[..decimal], &digits[decimal..]);
    }
    let fraction = if digits.len() == 1 { "0" } else { &digits[1..] };
    format!("{sign}{}.{fraction}E{exponent}", &digits[..1])
}

fn decompose_template(template: &str, arguments: &[String]) -> Option<String> {
    // TranslatableContents.decomposeTemplate rejects malformed formats and missing arguments.
    let bytes = template.as_bytes();
    let mut current = 0;
    let mut replacement_index = 0;
    let mut text = String::new();
    while let Some(offset) = template[current..].find('%') {
        let start = current + offset;
        text.push_str(&template[current..start]);
        current = start + 1;
        if bytes.get(current) == Some(&b'%') {
            text.push('%');
            current += 1;
            continue;
        }
        let digits = current;
        while bytes.get(current).is_some_and(u8::is_ascii_digit) {
            current += 1;
        }
        let index = if current > digits {
            if bytes.get(current) != Some(&b'$') {
                return None;
            }
            let index = template[digits..current]
                .parse::<i32>()
                .ok()?
                .checked_sub(1)?;
            current += 1;
            usize::try_from(index).ok()?
        } else {
            let index = replacement_index;
            replacement_index += 1;
            index
        };
        if bytes.get(current) != Some(&b's') {
            return None;
        }
        current += 1;
        text.push_str(arguments.get(index)?);
    }
    text.push_str(&template[current..]);
    Some(text)
}
