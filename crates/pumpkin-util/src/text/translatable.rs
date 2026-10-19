use super::{TextComponentBase, TextContent, component_codec::collection_tags};
use crate::translation::{
    Locale, Placeholder, get_translation, get_translation_with_fallback, placeholder_at,
};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};

const MAX_RENDERED_BYTES: usize = 32 * 1024;
const MAX_RENDER_STEPS: usize = 128 * 1024;
const MAX_RENDER_DEPTH: usize = 128;

pub(super) fn get_text(component: &NbtCompound, locale: Locale) -> String {
    let mut renderer = Renderer {
        text: String::new(),
        steps: MAX_RENDER_STEPS,
    };
    renderer.compound(component, locale, 0);
    renderer.text
}

pub(super) fn component_text(component: &TextComponentBase, locale: Locale) -> String {
    let mut renderer = Renderer {
        text: String::new(),
        steps: MAX_RENDER_STEPS,
    };
    renderer.component(component, locale, 0);
    renderer.text
}

struct Renderer {
    text: String,
    steps: usize,
}

impl Renderer {
    fn component(&mut self, component: &TextComponentBase, locale: Locale, depth: usize) {
        if !self.enter(depth) {
            return;
        }
        match component.content.as_ref() {
            TextContent::Translatable { component } => self.compound(&component.0, locale, depth),
            TextContent::Opaque { component } => self.compound(&component.0, Locale::EnUs, depth),
            TextContent::Text { text } => self.append(text),
            TextContent::Translate {
                translate,
                bedrock_translate,
                with,
            } => {
                let key = bedrock_translate.as_ref().unwrap_or(translate);
                let template = get_translation(&format!("minecraft:{key}"), locale);
                self.component_translation(&template, with, locale, depth);
            }
            TextContent::Custom { key, with, .. } => {
                let template = get_translation(key, locale);
                self.component_translation(&template, with, locale, depth);
            }
            TextContent::EntityNames { selector, .. } => self.append(selector),
            TextContent::Keybind { keybind } => self.append(keybind),
            TextContent::PlayerSprite { profile, .. } => {
                self.append(profile.0.get_string("name").unwrap_or_default());
            }
        }
        for child in &component.extra {
            if self.full() {
                break;
            }
            self.component(child, locale, depth + 1);
        }
    }

    fn component_translation(
        &mut self,
        template: &str,
        with: &[TextComponentBase],
        locale: Locale,
        depth: usize,
    ) {
        if with.is_empty() || !template.contains('%') {
            self.append(template);
            return;
        }
        self.steps = self.steps.saturating_sub(template.len());
        let bytes = template.as_bytes();
        let mut next = 0;
        let mut current = 0;
        let mut position = 0;
        while position < bytes.len() && !self.full() {
            if bytes[position] != b'%' || (position > 0 && bytes[position - 1] == b'\\') {
                position += 1;
                continue;
            }
            let Some((placeholder, end)) = placeholder_at(bytes, position) else {
                position += 1;
                continue;
            };
            self.append(&template[current..position]);
            match placeholder {
                Placeholder::Escape => self.append("%"),
                Placeholder::Next => {
                    self.component(&with[next], locale, depth + 1);
                    next = (next + 1).min(with.len() - 1);
                }
                Placeholder::Indexed(index) => {
                    self.component(&with[index.clamp(1, with.len()) - 1], locale, depth + 1);
                }
            }
            current = end + 1;
            position = current;
        }
        if !self.full() {
            self.append(&template[current..]);
        }
    }

    const fn full(&self) -> bool {
        self.text.len() >= MAX_RENDERED_BYTES || self.steps == 0
    }

    const fn enter(&mut self, depth: usize) -> bool {
        if depth >= MAX_RENDER_DEPTH {
            self.steps = 0;
        }
        if self.full() {
            return false;
        }
        self.steps -= 1;
        true
    }

    fn append(&mut self, value: &str) {
        let mut length = value.len().min(MAX_RENDERED_BYTES - self.text.len());
        while !value.is_char_boundary(length) {
            length -= 1;
        }
        self.text.push_str(&value[..length]);
        if length < value.len() {
            self.steps = 0;
        }
    }

    fn compound(&mut self, component: &NbtCompound, locale: Locale, depth: usize) {
        if !self.enter(depth) {
            return;
        }
        if let Some(key) = component.get_string("translate") {
            // TranslatableContents.decompose selects Language.getOrDefault first.
            let template = get_translation_with_fallback(
                &format!("minecraft:{key}"),
                locale,
                component.get_string("fallback").unwrap_or(key),
            );
            self.steps = self.steps.saturating_sub(template.len());
            if self.full() {
                return;
            }
            let arguments = component
                .get("with")
                .and_then(collection_tags)
                .unwrap_or_default();
            // Validate before emitting: malformed templates render literally in vanilla.
            if visit_template(&template, arguments.len(), |_, _| true).is_some() {
                let _ = visit_template(&template, arguments.len(), |literal, argument| {
                    if self.full() {
                        return false;
                    }
                    self.append(literal);
                    if let Some(index) = argument {
                        self.tag(&arguments[index], locale, depth + 1);
                    }
                    !self.full()
                });
            } else {
                self.append(&template);
            }
        } else {
            self.append(
                component
                    .get_string("text")
                    .or_else(|| component.get_string("fallback"))
                    .or_else(|| component.get_string("keybind"))
                    .or_else(|| component.get_string("selector"))
                    .unwrap_or_default(),
            );
        }
        if let Some(children) = component.get_list("extra") {
            for child in children {
                if self.full() {
                    break;
                }
                self.tag(child, locale, depth + 1);
            }
        }
    }

    fn tag(&mut self, tag: &NbtTag, locale: Locale, depth: usize) {
        if let NbtTag::Compound(component) = tag {
            if component.child_tags.len() == 1
                && let Some(value) = component.get("")
            {
                if self.enter(depth) {
                    self.tag(value, locale, depth + 1);
                }
            } else {
                self.compound(component, locale, depth);
            }
            return;
        }
        if !self.enter(depth) {
            return;
        }
        // TranslatableContents.getArgument uses Number.toString or visits a component.
        match tag {
            NbtTag::Byte(value) => self.append(&value.to_string()),
            NbtTag::Short(value) => self.append(&value.to_string()),
            NbtTag::Int(value) => self.append(&value.to_string()),
            NbtTag::Long(value) => self.append(&value.to_string()),
            NbtTag::Float(value) => self.append(&floating_point_text(format!("{value:?}"))),
            NbtTag::Double(value) => self.append(&floating_point_text(format!("{value:?}"))),
            NbtTag::String(value) => self.append(value),
            NbtTag::List(children) => {
                for child in children {
                    if self.full() {
                        break;
                    }
                    self.tag(child, locale, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn visit_template(
    template: &str,
    arguments: usize,
    mut visit: impl FnMut(&str, Option<usize>) -> bool,
) -> Option<()> {
    // TranslatableContents.decomposeTemplate rejects malformed formats and missing arguments.
    let bytes = template.as_bytes();
    let mut current = 0;
    let mut replacement_index = 0;
    while let Some(offset) = template[current..].find('%') {
        let start = current + offset;
        if !visit(&template[current..start], None) {
            return Some(());
        }
        current = start + 1;
        if bytes.get(current) == Some(&b'%') {
            if !visit("%", None) {
                return Some(());
            }
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
        if bytes.get(current) != Some(&b's') || index >= arguments {
            return None;
        }
        current += 1;
        if !visit("", Some(index)) {
            return Some(());
        }
    }
    visit(&template[current..], None);
    Some(())
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
