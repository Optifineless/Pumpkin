//! Shared byte, depth and traversal accounting, including opaque substitution copies.
pub const MAX_BYTES: usize = 32 * 1024;
pub const COMPONENT_OVERHEAD: usize = 192;
const MAX_STEPS: usize = 128 * 1024;
const MAX_DEPTH: usize = 128;

pub struct Budget {
    bytes: usize,
    steps: usize,
}

impl Budget {
    pub const fn new() -> Self {
        Self {
            bytes: MAX_BYTES,
            steps: MAX_STEPS,
        }
    }
    pub const fn full(&self) -> bool {
        self.bytes == 0 || self.steps == 0
    }
    pub const fn enter(&mut self, depth: usize) -> bool {
        if depth >= MAX_DEPTH {
            self.steps = 0;
        }
        if self.full() {
            return false;
        }
        self.steps -= 1;
        true
    }
    pub const fn scan(&mut self, bytes: usize) -> bool {
        self.steps = self.steps.saturating_sub(bytes);
        !self.full()
    }
    pub const fn reserve(&mut self, bytes: usize) -> bool {
        if bytes > self.bytes || self.steps == 0 {
            self.steps = 0;
            return false;
        }
        self.bytes -= bytes;
        true
    }
    pub fn text<'a>(&mut self, value: &'a str) -> &'a str {
        let mut end = value.len().min(self.bytes);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.bytes -= end;
        if end < value.len() {
            self.steps = 0;
        }
        &value[..end]
    }
}

impl Budget {
    pub fn compound(&mut self, value: &pumpkin_nbt::NbtCompound, depth: usize) -> bool {
        if !self.enter(depth) || !self.reserve(16) {
            return false;
        }
        value
            .child_tags
            .iter()
            .all(|(key, tag)| self.reserve(key.len()) && self.tag(tag, depth + 1))
    }

    fn tag(&mut self, value: &pumpkin_nbt::tag::NbtTag, depth: usize) -> bool {
        use pumpkin_nbt::tag::NbtTag;
        if let NbtTag::Compound(value) = value {
            return self.compound(value, depth);
        }
        if !self.enter(depth) || !self.reserve(16) {
            return false;
        }
        match value {
            NbtTag::String(value) => self.reserve(value.len()),
            NbtTag::List(values) => values.iter().all(|tag| self.tag(tag, depth + 1)),
            NbtTag::ByteArray(values) => self.reserve(values.len()) && self.scan(values.len()),
            NbtTag::IntArray(values) => {
                self.reserve(values.len().saturating_mul(4)) && self.scan(values.len())
            }
            NbtTag::LongArray(values) => {
                self.reserve(values.len().saturating_mul(8)) && self.scan(values.len())
            }
            _ => true,
        }
    }

    pub fn component(&mut self, value: &super::TextComponentBase, depth: usize) -> bool {
        if !self.enter(depth) || !self.reserve(COMPONENT_OVERHEAD) {
            return false;
        }
        self.content(&value.content, depth + 1)
            && value
                .extra
                .iter()
                .all(|child| self.component(child, depth + 1))
            && self.style(&value.style, depth + 1)
    }

    fn content(&mut self, value: &super::TextContent, depth: usize) -> bool {
        use super::TextContent;
        match value {
            TextContent::Text { text } => self.reserve(text.len()),
            TextContent::Translate {
                translate,
                bedrock_translate,
                with,
            } => {
                self.reserve(translate.len())
                    && bedrock_translate
                        .as_ref()
                        .is_none_or(|key| self.reserve(key.len()))
                    && with.iter().all(|child| self.component(child, depth))
            }
            TextContent::Custom { key, with, .. } => {
                self.reserve(key.len()) && with.iter().all(|child| self.component(child, depth))
            }
            TextContent::EntityNames {
                selector,
                separator,
            } => {
                self.reserve(selector.len())
                    && separator
                        .as_ref()
                        .is_none_or(|text| self.reserve(text.len()))
            }
            TextContent::Keybind { keybind } => self.reserve(keybind.len()),
            TextContent::Opaque { component } | TextContent::Translatable { component } => {
                self.compound(&component.0, depth)
            }
            TextContent::PlayerSprite {
                type_name, profile, ..
            } => self.reserve(type_name.len()) && self.compound(&profile.0, depth),
        }
    }

    fn style(&mut self, value: &super::style::Style, depth: usize) -> bool {
        use super::{click::ClickEvent, hover::HoverEvent};
        if !value
            .insertion
            .as_ref()
            .is_none_or(|text| self.reserve(text.len()))
            || !value
                .font
                .as_ref()
                .is_none_or(|text| self.reserve(text.len()))
        {
            return false;
        }
        let click = match value.click_event.as_ref() {
            Some(ClickEvent::OpenUrl { url }) => Some(url),
            Some(ClickEvent::OpenFile { path }) => Some(path),
            Some(ClickEvent::RunCommand { command } | ClickEvent::SuggestCommand { command }) => {
                Some(command)
            }
            Some(ClickEvent::CopyToClipboard { value }) => Some(value),
            _ => None,
        };
        if !click.is_none_or(|text| self.reserve(text.len())) {
            return false;
        }
        match value.hover_event.as_ref() {
            Some(HoverEvent::ShowText { value }) => {
                value.iter().all(|child| self.component(child, depth))
            }
            Some(HoverEvent::ShowEntity { id, uuid, name }) => {
                self.reserve(id.len())
                    && self.reserve(uuid.len())
                    && name.as_ref().is_none_or(|values| {
                        values.iter().all(|child| self.component(child, depth))
                    })
            }
            Some(HoverEvent::ShowItem { id, .. }) => self.reserve(id.len()),
            None => true,
        }
    }
}
