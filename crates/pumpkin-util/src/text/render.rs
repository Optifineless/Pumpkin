//! One budget for every server-side text expansion, including formatting bytes.
use super::{TextComponentBase, TextContent, click::ClickEvent, color::Color, style::Style};
use crate::{
    translation::{Locale, Placeholder, get_render_translation, placeholder_at},
    version::JavaMinecraftVersion,
};
use colored::Colorize;

pub(super) use super::render_budget::Budget;

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Plain,
    Console,
    BedrockRaw,
    Legacy {
        version: Option<JavaMinecraftVersion>,
        code: char,
    },
}

#[derive(Clone, Copy, Default)]
pub(super) struct ConsoleStyle<'a> {
    color: Option<Color>,
    bold: Option<bool>,
    italic: Option<bool>,
    underlined: Option<bool>,
    strikethrough: Option<bool>,
    click: Option<&'a ClickEvent>,
}

impl<'a> ConsoleStyle<'a> {
    fn inherit(style: &'a Style, parent: Self) -> Self {
        Self {
            color: style.color.or(parent.color),
            bold: style.bold.or(parent.bold),
            italic: style.italic.or(parent.italic),
            underlined: style.underlined.or(parent.underlined),
            strikethrough: style.strikethrough.or(parent.strikethrough),
            click: style.click_event.as_ref().or(parent.click),
        }
    }
    fn marker(self) -> String {
        // Derive constant-size ANSI affixes without formatting an expanded child String.
        let mut marker = self.color.map_or_else(
            || "x".to_owned(),
            |color| color.console_color("x").to_string(),
        );
        if self.bold.is_some() {
            marker = marker.bold().to_string();
        }
        if self.italic.is_some() {
            marker = marker.italic().to_string();
        }
        if self.underlined.is_some() {
            marker = marker.underline().to_string();
        }
        if self.strikethrough.is_some() {
            marker = marker.strikethrough().to_string();
        }
        marker
    }
}

pub(super) struct Renderer {
    pub(super) text: String,
    pub(super) budget: Budget,
    pub(super) mode: Mode,
}

pub(super) fn component(component: &TextComponentBase, locale: Locale, mode: Mode) -> String {
    let mut renderer = Renderer::new(mode);
    renderer.component(component, locale, 0, ConsoleStyle::default());
    renderer.text
}

pub fn translation(key: &str, locale: Locale, with: &[TextComponentBase], pretty: bool) -> String {
    let mut renderer = Renderer::new(if pretty { Mode::Console } else { Mode::Plain });
    let template = get_render_translation(key, locale, None);
    renderer.component_translation(&template, with, locale, 0, ConsoleStyle::default());
    renderer.text
}

impl Renderer {
    pub(super) const fn new(mode: Mode) -> Self {
        Self {
            text: String::new(),
            budget: Budget::new(),
            mode,
        }
    }
    pub(super) const fn full(&self) -> bool {
        self.budget.full()
    }
    pub(super) fn append(&mut self, value: &str, style: ConsoleStyle<'_>) {
        if self.full() || value.is_empty() {
            return;
        }
        if !matches!(self.mode, Mode::Console) {
            self.text.push_str(self.budget.text(value));
            return;
        }
        let marker = style.marker();
        let (prefix, suffix) = marker.split_once('x').unwrap_or(("", ""));
        let (link_prefix, link) = match style.click {
            Some(ClickEvent::OpenUrl { url }) => ("", Some(url.as_ref())),
            Some(ClickEvent::OpenFile { path }) => ("file://", Some(path.as_ref())),
            _ => ("", None),
        };
        let link_bytes = link.map_or(0, |link| {
            link.len()
                .saturating_add(link_prefix.len())
                .saturating_add(14)
        });
        if !self.budget.reserve(
            prefix
                .len()
                .saturating_add(suffix.len())
                .saturating_add(link_bytes),
        ) {
            return;
        }
        if let Some(link) = link {
            self.text.push_str("\x1b]8;;");
            self.text.push_str(link_prefix);
            self.text.push_str(link);
            self.text.push_str("\x1b\\");
        }
        self.text.push_str(prefix);
        self.text.push_str(self.budget.text(value));
        self.text.push_str(suffix);
        if link.is_some() {
            self.text.push_str("\x1b]8;;\x1b\\");
        }
    }

    pub(super) fn component(
        &mut self,
        component: &TextComponentBase,
        locale: Locale,
        depth: usize,
        parent: ConsoleStyle<'_>,
    ) {
        if !self.budget.enter(depth) {
            return;
        }
        let style = ConsoleStyle::inherit(&component.style, parent);
        if let Mode::Legacy { version, code } = self.mode {
            self.legacy_style(&component.style, version, code);
        }
        self.content(&component.content, locale, depth, style);
        for child in &component.extra {
            if self.full() {
                break;
            }
            self.component(child, locale, depth + 1, style);
            if let Mode::Legacy { code, .. } = self.mode {
                self.append(&format!("{code}r"), style);
            }
        }
    }

    fn content(
        &mut self,
        content: &TextContent,
        locale: Locale,
        depth: usize,
        style: ConsoleStyle<'_>,
    ) {
        match content {
            TextContent::Translatable { component } => {
                self.compound(&component.0, locale, depth, style);
            }
            TextContent::Opaque { component } => {
                self.compound(&component.0, Locale::EnUs, depth, style);
            }
            TextContent::Text { text } => self.append(text, style),
            TextContent::Translate {
                translate,
                bedrock_translate,
                with,
            } => {
                if matches!(self.mode, Mode::BedrockRaw) {
                    self.append("%", style);
                    self.append(bedrock_translate.as_deref().unwrap_or(translate), style);
                    return;
                }
                let key = match self.mode {
                    Mode::Plain => format!(
                        "minecraft:{}",
                        bedrock_translate.as_ref().unwrap_or(translate)
                    ),
                    Mode::Legacy { version: None, .. } => {
                        bedrock_translate.as_ref().unwrap_or(translate).to_string()
                    }
                    _ => format!("minecraft:{translate}"),
                };
                let template = get_render_translation(&key, locale, None);
                self.component_translation(&template, with, locale, depth, style);
            }
            TextContent::Custom { key, with, .. } => {
                if matches!(self.mode, Mode::BedrockRaw) {
                    self.append("%", style);
                    self.append(key, style);
                    return;
                }
                let template = get_render_translation(key, locale, None);
                self.component_translation(&template, with, locale, depth, style);
            }
            TextContent::EntityNames { selector, .. } => self.append(selector, style),
            TextContent::Keybind { keybind } => self.append(keybind, style),
            TextContent::PlayerSprite { profile, .. } => self.append(
                profile
                    .0
                    .get_string("name")
                    .unwrap_or(if matches!(self.mode, Mode::Console) {
                        "player_sprite"
                    } else {
                        ""
                    }),
                style,
            ),
        }
    }

    fn component_translation(
        &mut self,
        template: &str,
        with: &[TextComponentBase],
        locale: Locale,
        depth: usize,
        style: ConsoleStyle<'_>,
    ) {
        if !self.budget.scan(template.len()) {
            return;
        }
        visit_native_template(template, with.len(), |literal, index| {
            if self.full() {
                return false;
            }
            self.append(literal, style);
            if let Some(index) = index {
                self.argument(&with[index], locale, depth + 1, style);
            }
            !self.full()
        });
    }

    fn argument(
        &mut self,
        argument: &TextComponentBase,
        locale: Locale,
        depth: usize,
        style: ConsoleStyle<'_>,
    ) {
        let mode = self.mode;
        // Legacy translators have always rendered substitution contents as plain text.
        if matches!(mode, Mode::Legacy { .. }) {
            self.mode = Mode::Plain;
        }
        let locale = if matches!(mode, Mode::Console) {
            Locale::EnUs
        } else {
            locale
        };
        self.component(argument, locale, depth, style);
        self.mode = mode;
    }

    fn legacy_style(&mut self, style: &Style, version: Option<JavaMinecraftVersion>, code: char) {
        let plain = ConsoleStyle::default();
        if let Some(color) = style.color {
            match color {
                Color::Named(named) => {
                    self.append(&format!("{code}{}", named.to_legacy_char()), plain);
                }
                Color::Reset => self.append(&format!("{code}r"), plain),
                Color::Rgb(rgb) => {
                    if let Some(version) = version {
                        if version >= JavaMinecraftVersion::V_1_16 {
                            self.append(&format!("{code}x"), plain);
                            for digit in
                                format!("{:02x}{:02x}{:02x}", rgb.red, rgb.green, rgb.blue).chars()
                            {
                                self.append(&format!("{code}{digit}"), plain);
                            }
                        } else {
                            self.append(
                                &format!("{code}{}", rgb.to_nearest_named().to_legacy_char()),
                                plain,
                            );
                        }
                    }
                }
            }
        }
        for (enabled, letter) in [
            (style.bold, 'l'),
            (style.italic, 'o'),
            (style.underlined, 'n'),
            (style.strikethrough.filter(|_| version.is_some()), 'm'),
            (style.obfuscated, 'k'),
        ] {
            if enabled == Some(true) {
                self.append(&format!("{code}{letter}"), plain);
            }
        }
    }
}

/// Existing native placeholder rules, visiting borrowed arguments rather than cloning them.
pub(super) fn visit_native_template(
    template: &str,
    arguments: usize,
    mut visit: impl FnMut(&str, Option<usize>) -> bool,
) {
    if arguments == 0 || !template.contains('%') {
        visit(template, None);
        return;
    }
    let bytes = template.as_bytes();
    let (mut next, mut current, mut position) = (0, 0, 0);
    while position < bytes.len() {
        if bytes[position] != b'%' || (position > 0 && bytes[position - 1] == b'\\') {
            position += 1;
            continue;
        }
        let Some((placeholder, end)) = placeholder_at(bytes, position) else {
            position += 1;
            continue;
        };
        if !visit(&template[current..position], None) {
            return;
        }
        let (literal, argument) = match placeholder {
            Placeholder::Escape => ("%", None),
            Placeholder::Next => {
                let index = next;
                next = (next + 1).min(arguments - 1);
                ("", Some(index))
            }
            Placeholder::Indexed(index) => ("", Some(index.clamp(1, arguments) - 1)),
        };
        if !visit(literal, argument) {
            return;
        }
        current = end + 1;
        position = current;
    }
    visit(&template[current..], None);
}
