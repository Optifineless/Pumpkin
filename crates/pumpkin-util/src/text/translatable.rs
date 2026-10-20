use super::{
    TextComponentBase,
    render::{self, ConsoleStyle, Mode, Renderer},
};
use crate::translation::{Locale, get_render_translation};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};

#[cfg(test)]
pub(super) fn get_text(component: &NbtCompound, locale: Locale) -> String {
    let mut renderer = Renderer::new(Mode::Plain);
    renderer.compound(component, locale, 0, ConsoleStyle::default());
    renderer.text
}

pub(super) fn component_text(component: &TextComponentBase, locale: Locale) -> String {
    render::component(component, locale, Mode::Plain)
}

impl Renderer {
    pub(super) fn compound(
        &mut self,
        component: &NbtCompound,
        locale: Locale,
        depth: usize,
        style: ConsoleStyle<'_>,
    ) {
        if !self.budget.enter(depth) {
            return;
        }
        if let Some(key) = component.get_string("translate") {
            // TranslatableContents.decompose selects Language.getOrDefault first.
            let template = get_render_translation(
                &format!("minecraft:{key}"),
                locale,
                Some(component.get_string("fallback").unwrap_or(key)),
            );
            if !self.budget.scan(template.len()) {
                return;
            }
            let arguments = component.get("with");
            let count = match arguments {
                Some(NbtTag::List(values)) => values.len(),
                Some(NbtTag::ByteArray(values)) => values.len(),
                Some(NbtTag::IntArray(values)) => values.len(),
                Some(NbtTag::LongArray(values)) => values.len(),
                _ => 0,
            };
            // Validate before emitting: malformed templates render literally in vanilla.
            if visit_template(&template, count, |_, _| true).is_some() {
                let _ = visit_template(&template, count, |literal, argument| {
                    if self.full() {
                        return false;
                    }
                    self.append(literal, style);
                    if let Some(index) = argument {
                        match arguments {
                            Some(NbtTag::List(values)) => {
                                self.tag(&values[index], locale, depth + 1, style);
                            }
                            Some(NbtTag::ByteArray(values)) => {
                                self.tag(&NbtTag::Byte(values[index]), locale, depth + 1, style);
                            }
                            Some(NbtTag::IntArray(values)) => {
                                self.tag(&NbtTag::Int(values[index]), locale, depth + 1, style);
                            }
                            Some(NbtTag::LongArray(values)) => {
                                self.tag(&NbtTag::Long(values[index]), locale, depth + 1, style);
                            }
                            _ => {}
                        }
                    }
                    !self.full()
                });
            } else {
                self.append(&template, style);
            }
        } else {
            self.append(
                component
                    .get_string("text")
                    .or_else(|| component.get_string("fallback"))
                    .or_else(|| component.get_string("keybind"))
                    .or_else(|| component.get_string("selector"))
                    .unwrap_or_default(),
                style,
            );
        }
        if let Some(children) = component.get_list("extra") {
            for child in children {
                if self.full() {
                    break;
                }
                self.tag(child, locale, depth + 1, style);
            }
        }
    }

    fn tag(&mut self, tag: &NbtTag, locale: Locale, depth: usize, style: ConsoleStyle<'_>) {
        if let NbtTag::Compound(component) = tag {
            if component.child_tags.len() == 1
                && let Some(value) = component.get("")
            {
                if self.budget.enter(depth) {
                    self.tag(value, locale, depth + 1, style);
                }
            } else {
                self.compound(component, locale, depth, style);
            }
            return;
        }
        if !self.budget.enter(depth) {
            return;
        }
        // TranslatableContents.getArgument uses Number.toString or visits a component.
        match tag {
            NbtTag::Byte(value) => self.append(&value.to_string(), style),
            NbtTag::Short(value) => self.append(&value.to_string(), style),
            NbtTag::Int(value) => self.append(&value.to_string(), style),
            NbtTag::Long(value) => self.append(&value.to_string(), style),
            NbtTag::Float(value) => self.append(&floating_point_text(format!("{value:?}")), style),
            NbtTag::Double(value) => self.append(&floating_point_text(format!("{value:?}")), style),
            NbtTag::String(value) => self.append(value, style),
            NbtTag::List(children) => {
                for child in children {
                    if self.full() {
                        break;
                    }
                    self.tag(child, locale, depth + 1, style);
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
