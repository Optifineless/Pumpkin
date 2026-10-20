//! Bounded custom-translation materialization for the public text encoders.
use super::render_budget::COMPONENT_OVERHEAD;
use super::{
    TextComponent, TextComponentBase, TextContent,
    click::ClickEvent,
    hover::HoverEvent,
    render::{Budget, visit_native_template},
    style::Style,
};
use crate::translation::{Locale, get_render_translation};

pub(super) fn needs_translation(value: &TextComponentBase) -> bool {
    // Inspect existing input only, without recursion, cloning or expansion. Native
    // client-side components also pass through entity/text-display save encoders.
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value.content.as_ref() {
            TextContent::Custom { .. } => return true,
            TextContent::Translate { with, .. } => pending.extend(with),
            _ => {}
        }
        pending.extend(&value.extra);
        match value.style.hover_event.as_ref() {
            Some(HoverEvent::ShowText { value }) => pending.extend(value),
            Some(HoverEvent::ShowEntity {
                name: Some(values), ..
            }) => pending.extend(values),
            _ => {}
        }
    }
    false
}

pub(super) fn component(value: &TextComponentBase) -> TextComponentBase {
    materialize(value, &mut Budget::new(), 0)
}

fn literal(text: String) -> TextComponentBase {
    TextComponent::text(text).0
}

pub(super) fn materialize(
    value: &TextComponentBase,
    budget: &mut Budget,
    depth: usize,
) -> TextComponentBase {
    // Include structural overhead so empty substitutions cannot create an enormous tree.
    if !budget.enter(depth) || !budget.reserve(COMPONENT_OVERHEAD) {
        return literal(String::new());
    }
    let mut extra = Vec::new();
    let content = match value.content.as_ref() {
        TextContent::Text { text } => TextContent::Text {
            text: budget.text(text).to_owned().into(),
        },
        TextContent::Custom { key, with, locale } => {
            custom(key, with, *locale, budget, depth, &mut extra)
        }
        TextContent::Translate {
            translate,
            bedrock_translate,
            with,
        } => {
            if budget.reserve(translate.len())
                && bedrock_translate
                    .as_ref()
                    .is_none_or(|key| budget.reserve(key.len()))
            {
                TextContent::Translate {
                    translate: translate.clone(),
                    bedrock_translate: bedrock_translate.clone(),
                    with: children(with, budget, depth + 1),
                }
            } else {
                TextContent::Text { text: "".into() }
            }
        }
        TextContent::EntityNames {
            selector,
            separator,
        } => {
            if budget.reserve(selector.len())
                && separator
                    .as_ref()
                    .is_none_or(|text| budget.reserve(text.len()))
            {
                TextContent::EntityNames {
                    selector: selector.clone(),
                    separator: separator.clone(),
                }
            } else {
                TextContent::Text { text: "".into() }
            }
        }
        TextContent::Keybind { keybind } => {
            if budget.reserve(keybind.len()) {
                TextContent::Keybind {
                    keybind: keybind.clone(),
                }
            } else {
                TextContent::Text { text: "".into() }
            }
        }
        content @ (TextContent::Opaque { component } | TextContent::Translatable { component }) => {
            if budget.compound(&component.0, depth + 1) {
                content.clone()
            } else {
                TextContent::Text { text: "".into() }
            }
        }
        content @ TextContent::PlayerSprite {
            type_name, profile, ..
        } => {
            if budget.reserve(type_name.len()) && budget.compound(&profile.0, depth + 1) {
                content.clone()
            } else {
                TextContent::Text { text: "".into() }
            }
        }
    };
    extra.extend(children(&value.extra, budget, depth + 1));
    TextComponentBase {
        content: Box::new(content),
        style: Box::new(style(&value.style, budget, depth + 1)),
        extra,
    }
}

fn custom(
    key: &str,
    with: &[TextComponentBase],
    locale: Locale,
    budget: &mut Budget,
    depth: usize,
    extra: &mut Vec<TextComponentBase>,
) -> TextContent {
    let template = get_render_translation(key, locale, None);
    let mut first = None;
    if budget.scan(template.len()) {
        visit_native_template(&template, with.len().max(1), |text, argument| {
            if budget.full() {
                return false;
            }
            if first.is_none() {
                first = Some(budget.text(text).to_owned());
            } else if !text.is_empty() && budget.reserve(COMPONENT_OVERHEAD) {
                extra.push(literal(budget.text(text).to_owned()));
            }
            if let Some(argument) = argument.and_then(|index| with.get(index)) {
                if budget.full() {
                    return false;
                }
                extra.push(materialize(argument, budget, depth + 1));
            }
            !budget.full()
        });
    }
    TextContent::Text {
        text: first.unwrap_or_default().into(),
    }
}

fn children(
    values: &[TextComponentBase],
    budget: &mut Budget,
    depth: usize,
) -> Vec<TextComponentBase> {
    let mut result = Vec::new();
    for value in values {
        if budget.full() {
            break;
        }
        result.push(materialize(value, budget, depth));
    }
    result
}

fn style(value: &Style, budget: &mut Budget, depth: usize) -> Style {
    Style {
        color: value.color,
        bold: value.bold,
        italic: value.italic,
        underlined: value.underlined,
        strikethrough: value.strikethrough,
        obfuscated: value.obfuscated,
        shadow_color: value.shadow_color,
        insertion: value
            .insertion
            .as_ref()
            .and_then(|text| atomic_text(text, budget)),
        font: value
            .font
            .as_ref()
            .and_then(|text| atomic_text(text, budget)),
        click_event: value
            .click_event
            .as_ref()
            .and_then(|event| click(event, budget)),
        hover_event: value
            .hover_event
            .as_ref()
            .and_then(|event| hover(event, budget, depth)),
    }
}

fn atomic_text(value: &str, budget: &mut Budget) -> Option<String> {
    budget.reserve(value.len()).then(|| value.to_owned())
}

fn click(value: &ClickEvent, budget: &mut Budget) -> Option<ClickEvent> {
    let bytes = match value {
        ClickEvent::OpenUrl { url } => url.len(),
        ClickEvent::OpenFile { path } => path.len(),
        ClickEvent::RunCommand { command } | ClickEvent::SuggestCommand { command } => {
            command.len()
        }
        ClickEvent::CopyToClipboard { value } => value.len(),
        ClickEvent::ChangePage { .. } => 0,
    };
    // ClickEvent codecs interpret the complete payload, never a string prefix.
    (budget.reserve(COMPONENT_OVERHEAD) && budget.reserve(bytes)).then(|| value.clone())
}

fn hover(value: &HoverEvent, budget: &mut Budget, depth: usize) -> Option<HoverEvent> {
    if !budget.enter(depth) || !budget.reserve(COMPONENT_OVERHEAD) {
        return None;
    }
    let event = match value {
        HoverEvent::ShowText { value } => HoverEvent::ShowText {
            value: children(value, budget, depth + 1),
        },
        HoverEvent::ShowEntity { id, uuid, name } => {
            // HoverEvent.EntityTooltipInfo.CODEC requires the original registry ID and UUID.
            if !budget.reserve(id.len()) || !budget.reserve(uuid.len()) {
                return None;
            }
            HoverEvent::ShowEntity {
                id: id.clone(),
                uuid: uuid.clone(),
                name: name
                    .as_ref()
                    .map(|values| children(values, budget, depth + 1)),
            }
        }
        HoverEvent::ShowItem { id, count } => {
            if !budget.reserve(id.len()) {
                return None;
            }
            HoverEvent::ShowItem {
                id: id.clone(),
                count: *count,
            }
        }
    };
    // A partially materialized hover name/text is not a complete event payload.
    (!budget.full()).then_some(event)
}
