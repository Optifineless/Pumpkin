//! Preserve native input while bounding only generated Custom translation copies.
use super::{TextComponentBase, TextContent, hover::HoverEvent, render_budget::Budget, translated};

pub(super) fn component(value: TextComponentBase) -> TextComponentBase {
    if translated::needs_translation(&value) {
        materialize(value, &mut Budget::new())
    } else {
        value
    }
}

fn materialize(mut value: TextComponentBase, budget: &mut Budget) -> TextComponentBase {
    if matches!(value.content.as_ref(), TextContent::Custom { .. }) {
        // Only substitution copies are output growth. Original extras and style
        // retain their storage meaning even after the expansion budget is full.
        let extra = std::mem::take(&mut value.extra);
        let style = std::mem::take(&mut value.style);
        value = translated::materialize(&value, budget, 0);
        value.style = style;
        value.extra.extend(extra);
    }
    if let TextContent::Translate { with, .. } = value.content.as_mut() {
        *with = children(std::mem::take(with), budget);
    }
    value.extra = children(value.extra, budget);
    match value.style.hover_event.as_mut() {
        Some(HoverEvent::ShowText { value }) => {
            *value = children(std::mem::take(value), budget);
        }
        Some(HoverEvent::ShowEntity {
            name: Some(values), ..
        }) => {
            *values = children(std::mem::take(values), budget);
        }
        _ => {}
    }
    value
}

fn children(values: Vec<TextComponentBase>, budget: &mut Budget) -> Vec<TextComponentBase> {
    values
        .into_iter()
        .map(|value| materialize(value, budget))
        .collect()
}
