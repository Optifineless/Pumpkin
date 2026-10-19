//! Bounded custom-translation materialization for the public text encoders.
use super::render_budget::COMPONENT_OVERHEAD;
use super::{
    TextComponent, TextComponentBase, TextContent,
    click::ClickEvent,
    hover::HoverEvent,
    render::{Budget, visit_native_template},
    style::Style,
};
use crate::translation::get_render_translation;

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

fn materialize(value: &TextComponentBase, budget: &mut Budget, depth: usize) -> TextComponentBase {
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
            let template = get_render_translation(key, *locale, None);
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
        TextContent::Translate {
            translate,
            bedrock_translate,
            with,
        } => TextContent::Translate {
            translate: budget.text(translate).to_owned().into(),
            bedrock_translate: bedrock_translate
                .as_ref()
                .map(|key| budget.text(key).to_owned().into()),
            with: children(with, budget, depth + 1),
        },
        TextContent::EntityNames {
            selector,
            separator,
        } => TextContent::EntityNames {
            selector: budget.text(selector).to_owned().into(),
            separator: separator
                .as_ref()
                .map(|text| budget.text(text).to_owned().into()),
        },
        TextContent::Keybind { keybind } => TextContent::Keybind {
            keybind: budget.text(keybind).to_owned().into(),
        },
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
            .map(|text| budget.text(text).to_owned()),
        font: value.font.as_ref().map(|text| budget.text(text).to_owned()),
        click_event: value.click_event.as_ref().map(|event| click(event, budget)),
        hover_event: value
            .hover_event
            .as_ref()
            .and_then(|event| hover(event, budget, depth)),
    }
}

fn click(value: &ClickEvent, budget: &mut Budget) -> ClickEvent {
    match value {
        ClickEvent::OpenUrl { url } => ClickEvent::OpenUrl {
            url: budget.text(url).to_owned().into(),
        },
        ClickEvent::OpenFile { path } => ClickEvent::OpenFile {
            path: budget.text(path).to_owned().into(),
        },
        ClickEvent::RunCommand { command } => ClickEvent::RunCommand {
            command: budget.text(command).to_owned().into(),
        },
        ClickEvent::SuggestCommand { command } => ClickEvent::SuggestCommand {
            command: budget.text(command).to_owned().into(),
        },
        ClickEvent::CopyToClipboard { value } => ClickEvent::CopyToClipboard {
            value: budget.text(value).to_owned().into(),
        },
        ClickEvent::ChangePage { page } => ClickEvent::ChangePage { page: *page },
    }
}

fn hover(value: &HoverEvent, budget: &mut Budget, depth: usize) -> Option<HoverEvent> {
    if !budget.enter(depth) {
        return None;
    }
    Some(match value {
        HoverEvent::ShowText { value } => HoverEvent::ShowText {
            value: children(value, budget, depth + 1),
        },
        HoverEvent::ShowEntity { id, uuid, name } => HoverEvent::ShowEntity {
            id: budget.text(id).to_owned().into(),
            uuid: budget.text(uuid).to_owned().into(),
            name: name
                .as_ref()
                .map(|values| children(values, budget, depth + 1)),
        },
        HoverEvent::ShowItem { id, count } => HoverEvent::ShowItem {
            id: budget.text(id).to_owned().into(),
            count: *count,
        },
    })
}
