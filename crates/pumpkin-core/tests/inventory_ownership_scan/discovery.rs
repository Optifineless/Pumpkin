use super::source::{Token, tokenize};
use super::{Candidate, Record, RecordSink, record};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
struct Span {
    start: usize,
    open: usize,
    end: usize,
    kind: &'static str,
    name: String,
}

#[rustfmt::skip]
const STATE_FIELDS: &[&str] = &[
    "main_inventory", "selected_item_slot", "selected_slot", "equipment", "ender_chest", "item_use_time", "item_in_use", "active_hand", "using_item", "item_use_start_time", "using_hand", "cooldowns", "experience_level", "experience_progress", "experience_points", "experience_pick_up_delay", "xp_seed", "enchantment_seed", "attributes", "equipment_modifiers", "pending", "publishing", "inventory_changed", "storage_session", "carried_item", "game_mode", "abilities", "permission_level", "hunger_manager", "current_screen_handler", "selected_screen_handler", "open_container", "open_container_pos", "closing_menus", "close_claims", "window_id", "screen_handler_sync_id", "slots", "cursor", "tracked_slots", "tracked_cursor", "revision", "properties", "container_slots", "drag_status", "drag_button", "selected_slots", "sync_handler", "disable_sync", "crafting_inventory", "result", "recipe_provider", "offers", "active_offer", "selected_offer", "result_taken", "cost", "repair_count", "rename_only", "working_seed", "bookshelf_count", "pending_result", "offer_revision", "last_input", "last_interacted_slot", "disabled_slots", "crafting_ticks", "triggered", "recipes_used", "recipe_world", "last_potion_count", "ingredient_item", "ingredient_identity", "record", "playback", "book", "page", "payment", "contents", "items", "inventory", "stack", "never_despawn", "never_pickup", "admiring_item", "trading_player", "merchant_session", "cloud_state", "damage_owner", "lifecycle", "pending_hurt", "committed_admission", "snapshot", "viewer_count", "viewer_tracker", "drops_claimed", "pickup_allowed", "fixed", "drop_chance", "saddle", "radius", "cooldown", "restock_time", "restock_counter", "experience", "container_lock", "dirty", "comparator_update", "loot_key", "loot_table", "loot_seed", "cooking_time", "cooking_total_time", "brew_time", "fuel", "last_potion_count", "record_playback", "shards", "sherds", "facing", "game_time", "pickup_delay", "age", "health", "count", "value", "following_player", "weapon", "rotation", "visible", "drop_chance", "block_state", "pending_token", "incarnation", "removed_reason", "restock_timer", "update_flags", "trading", "xp", "experience", "seed", "inventory_changed", "callback", "listener", "listeners", "on_trade", "on_trade_updated", "on_close", "validity_check",
];

#[rustfmt::skip]
const MUTATORS: &[&str] = &[
    "set_stack", "set_stack_in_hand", "set_held_item", "remove_stack", "remove_item", "split", "clear", "clear_inventory", "insert_stack", "insert_stack_anywhere", "add_stack", "add_stack_to_slot", "offer", "swap", "swap_stack_with_hotbar", "update_slot", "update_stack", "set_item", "set_items", "take_stack", "take_item", "push", "pop", "insert", "remove", "swap_remove", "drain", "retain", "extend", "damage", "hurt_and_break", "shrink", "grow", "set_component", "remove_component", "apply_components", "replace", "read_nbt", "read_nbt_non_mut", "set_data", "set_contents", "set_inventory", "set_offers", "add_experience", "set_experience", "store", "fetch_add", "fetch_sub", "fetch_or", "fetch_and", "fetch_xor", "fetch_max", "fetch_min", "fetch_update", "compare_exchange", "compare_exchange_weak",
];

#[rustfmt::skip]
const NBT_COMPONENT: &[&str] = &[
    "read_nbt", "read_nbt_non_mut", "write_nbt", "from_nbt", "to_nbt", "set_data", "apply_components", "read_components", "set_component", "remove_component", "deserialize", "serialize",
];

#[rustfmt::skip]
const BOUNDARIES: &[&str] = &[
    "send_packet", "send_packets", "write_packet", "send", "on_closed", "on_close", "on_trade", "on_trade_updated", "validity_check", "callback", "listener", "get_property", "set_property", "get_stack", "take_stack", "can_insert", "can_take_items", "insert_stack", "fire", "call_event", "dispatch_event", "dispatch", "on_take", "on_craft", "on_quick_move", "get_available_recipes", "get_recipe", "block_on", "block_in_place", "spawn_blocking", "join", "await",
];

#[rustfmt::skip]
const CALLBACK_FIELDS: &[&str] = &["on_trade", "on_trade_updated", "on_close", "validity_check", "callback", "listener", "listeners"];

#[rustfmt::skip]
const LIFECYCLE: &[&str] = &[
    "remove", "retire", "unload", "on_chunk_unload", "despawn", "death", "die", "close", "on_closed", "save", "snapshot", "write_nbt", "from_nbt", "promote", "publish", "replace", "on_block_replaced", "set_block_entity", "set_block_entity_nbt", "set_data",
];

#[rustfmt::skip]
const AMBIGUOUS_FIELDS: &[&str] = &["items", "inventory", "stack", "count", "value", "age", "health", "experience", "seed", "result", "record", "playback", "book", "page", "contents", "slots", "cursor", "properties", "pending", "snapshot", "visible", "fixed", "radius", "cooldown", "fuel", "dirty", "trading", "saddle", "weapon", "facing", "rotation", "drop_chance", "lifecycle", "inventory_changed", "game_time", "pickup_delay", "block_state", "pending_token", "incarnation", "removed_reason", "restock_timer", "update_flags", "xp", "comparator_update", "container_lock", "viewer_count", "viewer_tracker", "drops_claimed", "pickup_allowed", "recipe_world", "last_potion_count", "triggered"];

fn is_mapped_state_field(name: &str) -> bool {
    STATE_FIELDS.contains(&name) && !AMBIGUOUS_FIELDS.contains(&name)
}

#[rustfmt::skip]
fn is_test_file(path: &Path) -> bool {
    path.components().any(|part| part.as_os_str() == "tests") || path.file_stem().is_some_and(|name| { let name = name.to_string_lossy(); name == "tests" || name.ends_with("_tests") || name.starts_with("test_") })
}

fn gather(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if !is_test_file(&path) {
                gather(&path, root, out)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_file(&path) {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

#[rustfmt::skip]
fn matching(tokens: &[Token<'_>], start: usize, open: &str, close: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(start) {
        if token.text == open { depth += 1; }
        else if token.text == close { depth = depth.checked_sub(1)?; if depth == 0 { return Some(i + 1); }
        }
    }
    None
}

#[rustfmt::skip]
fn skip_test_modules<'source>(tokens: &[Token<'source>]) -> Vec<Token<'source>> {
    let mut excluded = BTreeSet::new(); let mut i = 0;
    while i + 2 < tokens.len() {
        if tokens[i].text != "#" || tokens[i + 1].text != "[" { i += 1; continue; }
        let Some(attr_end) = matching(tokens, i + 1, "[", "]") else { i += 1; continue; };
        let attrs = &tokens[i + 2..attr_end - 1];
        let test_only = attrs.windows(3).any(|part| part[0].text == "cfg" && part[1].text == "(" && part[2].text == "test");
        if !test_only { i = attr_end; continue; }
        let mut body = attr_end;
        while body < tokens.len() && tokens[body].text != "{" && tokens[body].text != ";" { body += 1; }
        let end = if tokens.get(body).is_some_and(|token| token.text == "{") { matching(tokens, body, "{", "}").unwrap_or(body + 1) } else { body.saturating_add(1).min(tokens.len()) };
        excluded.extend(i..end); i = end.max(attr_end);
    }
    tokens.iter().enumerate().filter(|(i, _)| !excluded.contains(i)).map(|(_, token)| token.clone()).collect()
}

#[rustfmt::skip]
fn item_open(tokens: &[Token<'_>], from: usize) -> Option<usize> {
    let (mut round, mut square, mut angle) = (0usize, 0usize, 0usize);
    for (i, token) in tokens.iter().enumerate().skip(from) {
        match token.text {
            "(" => round += 1, ")" => round = round.saturating_sub(1), "[" => square += 1, "]" => square = square.saturating_sub(1),
            "<" => angle += 1, ">" => angle = angle.saturating_sub(1), "{" if round == 0 && square == 0 && angle == 0 => return Some(i),
            ";" if round == 0 && square == 0 && angle == 0 => return None, _ => {}
        }
    }
    None
}

#[rustfmt::skip]
fn item_spans(tokens: &[Token<'_>]) -> Vec<Span> {
    let mut spans = Vec::new();
    for i in 0..tokens.len() {
        let (kind, name_index, body_from) = match tokens[i].text {
            "fn" if tokens.get(i + 1).is_some_and(|t| is_ident(t.text)) => ("function", i + 1, i + 2),
            "struct" | "enum" | "union" | "trait" if tokens.get(i + 1).is_some() => ("type", i + 1, i + 2),
            "impl" => {
                let Some(open) = item_open(tokens, i + 1) else { continue; };
                let name = impl_name(tokens, i, open);
                let Some(end) = matching(tokens, open, "{", "}") else { continue; };
                spans.push(Span { start: i, open, end, kind: "impl", name });
                continue;
            }
            "macro_rules" if tokens.get(i + 1).is_some_and(|t| t.text == "!") => {
                let Some(name) = tokens.get(i + 2) else { continue; };
                let Some(open) = (i + 3..tokens.len()).find(|j| tokens[*j].text == "{") else { continue; };
                let Some(end) = matching(tokens, open, "{", "}") else { continue; };
                spans.push(Span { start: i, open, end, kind: "macro", name: name.text.to_owned() });
                continue;
            }
            _ => continue,
        };
        let Some(name) = tokens.get(name_index) else { continue; };
        let Some(open) = item_open(tokens, body_from) else { continue; };
        let Some(end) = matching(tokens, open, "{", "}") else { continue; };
        spans.push(Span { start: i, open, end, kind, name: name.text.to_owned() });
    }
    spans.sort_by_key(|span| (span.start, span.end));
    spans
}

fn is_ident(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
}

fn impl_name(tokens: &[Token<'_>], start: usize, open: usize) -> String {
    let end = (start + 1..open)
        .find(|i| tokens[*i].text == "where")
        .unwrap_or(open);
    let header = tokens[start + 1..end]
        .iter()
        .map(|token| token.text)
        .collect::<Vec<_>>()
        .join(" ");
    if header.is_empty() {
        "impl <unknown>".into()
    } else {
        format!("impl {header}")
    }
}

#[rustfmt::skip]
fn field_segments(tokens: &[Token<'_>], open: usize, end: usize) -> Vec<(usize, usize)> {
    let (mut start, mut braces, mut brackets, mut parens, mut angles) =
        (open + 1, 0usize, 0usize, 0usize, 0usize);
    let mut out = Vec::new();
    for (i, token) in tokens
        .iter()
        .enumerate()
        .take(end.saturating_sub(1))
        .skip(open + 1)
    {
        match token.text {
            "{" => braces += 1,
            "}" => braces = braces.saturating_sub(1),
            "[" => brackets += 1,
            "]" => brackets = brackets.saturating_sub(1),
            "(" => parens += 1,
            ")" => parens = parens.saturating_sub(1),
            "<" => angles += 1,
            ">" => angles = angles.saturating_sub(1),
            "," if braces == 0 && brackets == 0 && parens == 0 && angles == 0 => { out.push((start, i)); start = i + 1; }
            _ => {}
        }
    }
    if start < end.saturating_sub(1) {
        out.push((start, end - 1));
    }
    out
}

#[rustfmt::skip]
fn stack_aliases(tokens: &[Token<'_>]) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    for i in 0..tokens.len() {
        if tokens[i].text == "type" {
            if !tokens.get(i + 1).is_some_and(|token| is_ident(token.text))
                || tokens
                    .get(i + 2)
                    .is_none_or(|token| token.text != "=")
            {
                continue;
            }
            let Some(name) = tokens.get(i + 1) else { continue; };
            let end = (i + 3..tokens.len()).find(|j| tokens[*j].text == ";").unwrap_or(tokens.len());
            if tokens[i + 3..end].iter().any(|token| token.text == "ItemStack") { aliases.insert(name.text.to_owned()); }
        } else if tokens[i].text == "use" {
            let end = (i + 1..tokens.len()).find(|j| tokens[*j].text == ";").unwrap_or(tokens.len());
            for j in i + 1..end.saturating_sub(2) {
                if tokens[j].text == "ItemStack" && tokens[j + 1].text == "as" { aliases.insert(tokens[j + 2].text.to_owned()); }
            }
        }
    }
    aliases
}

fn field_name(tokens: &[Token<'_>], start: usize, end: usize) -> (String, usize) {
    let colon = (start..end).find(|i| tokens[*i].text == ":");
    colon.map_or_else(
        || (format!("tuple_field_{start}"), start),
        |colon| {
            let name = (start..colon)
                .rev()
                .find(|i| is_ident(tokens[*i].text))
                .map_or_else(|| format!("field{start}"), |i| tokens[i].text.to_owned());
            (name, colon + 1)
        },
    )
}

#[rustfmt::skip]
fn is_stack_type(tokens: &[Token<'_>], aliases: &BTreeSet<String>) -> bool { tokens.iter().any(|token| token.text == "ItemStack" || aliases.contains(token.text)) }

fn mapped_type_names(
    tokens: &[Token<'_>],
    spans: &[Span],
    aliases: &BTreeSet<String>,
) -> BTreeSet<String> {
    spans
        .iter()
        .filter(|span| span.kind == "type" && tokens[span.start].text == "struct")
        .filter(|span| {
            span.name == "ItemStack"
                || field_segments(tokens, span.open, span.end)
                    .iter()
                    .any(|(start, end)| {
                        let (name, type_start) = field_name(tokens, *start, *end);
                        is_stack_type(&tokens[type_start..*end], aliases)
                            || is_mapped_state_field(&name)
                    })
        })
        .map(|span| span.name.clone())
        .collect()
}

fn relevant_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [
        "inventory",
        "item",
        "player",
        "entity",
        "block",
        "merchant",
        "screen",
        "command",
        "plugin",
        "wasm",
        "world",
        "net/",
    ]
    .iter()
    .any(|part| p.contains(part))
}

#[rustfmt::skip]
fn is_call(tokens: &[Token<'_>], index: usize) -> bool { tokens.get(index + 1).is_some_and(|token| matches!(token.text, "(" | "!")) }

fn enclosing(span: &Span, parents: &[Span]) -> String {
    let mut ancestors: Vec<_> = parents
        .iter()
        .filter(|candidate| candidate.start < span.start && candidate.end >= span.end)
        .collect();
    ancestors.sort_by_key(|candidate| candidate.start);
    let mut parts: Vec<_> = ancestors
        .into_iter()
        .filter(|candidate| matches!(candidate.kind, "impl" | "type" | "function"))
        .map(|candidate| candidate.name.as_str())
        .collect();
    parts.push(match span.kind {
        "macro" => return format!("macro_rules!{}", span.name),
        "impl" => return format!("impl {}", span.name),
        _ => span.name.as_str(),
    });
    parts.join("::")
}

#[rustfmt::skip]
fn line_for(tokens: &[Token<'_>], index: usize) -> usize { tokens.get(index).map_or(1, |token| token.line) }

#[derive(Default)]
struct MethodSites {
    fields: Vec<usize>,
    mutators: Vec<usize>,
    nbt: Vec<usize>,
    boundaries: Vec<usize>,
    lifecycle: Vec<usize>,
    stack_site: bool,
}

#[rustfmt::skip]
fn method_sites(body: &[Token<'_>], name: &str, fields: &BTreeSet<String>, aliases: &BTreeSet<String>, mapped_path: bool, mapped_impl: bool) -> MethodSites {
    let mut sites = MethodSites { stack_site: body.iter().any(|token| token.text == "ItemStack" || aliases.contains(token.text)), ..MethodSites::default() };
    sites.fields = body.iter().enumerate().filter(|(index, token)| fields.contains(token.text)
        && body.get(index.wrapping_sub(1)).is_some_and(|prev| prev.text == ".")
        && (sites.stack_site || mapped_impl || mapped_path))
        .map(|(index, _)| index).collect();
    for (index, token) in body.iter().enumerate() {
        if MUTATORS.contains(&token.text) && (is_call(body, index) || token.text == name) { sites.mutators.push(index); }
        if NBT_COMPONENT.contains(&token.text) && (is_call(body, index) || token.text == name) { sites.nbt.push(index); }
        if BOUNDARIES.contains(&token.text) && (token.text == "await" || is_call(body, index)) { sites.boundaries.push(index); }
        if LIFECYCLE.contains(&token.text) && (is_call(body, index) || token.text == name) { sites.lifecycle.push(index); }
    }
    sites
}

#[rustfmt::skip]
fn scan_method(path: &str, tokens: &[Token<'_>], spans: &[Span], span: &Span, fields: &BTreeSet<String>, aliases: &BTreeSet<String>, mapped_types: &BTreeSet<String>) -> Vec<Record> {
    let item = enclosing(span, spans);
    let fingerprint = &tokens[span.start..span.end];
    let body: Vec<_> = fingerprint.iter().enumerate().filter(|(index, _)| !spans.iter().any(|nested| {
        nested.kind == "function" && span.start < nested.start && nested.end <= span.end
            && nested.start - span.start <= *index && *index < nested.end - span.start
    })).map(|(_, token)| token.clone()).collect();
    let mapped_impl = spans.iter().filter(|candidate| candidate.kind == "impl" && candidate.start < span.start && candidate.end >= span.end)
        .min_by_key(|candidate| candidate.end - candidate.start).is_some_and(|implementation| {
            mapped_types.contains(&implementation.name) || tokens[implementation.start..implementation.open].iter()
                .any(|token| matches!(token.text, "Inventory" | "Slot" | "Container" | "ScreenHandler" | "Merchant")) });
    let sites = method_sites(&body, &span.name, fields, aliases, relevant_path(path), mapped_impl);
    let scoped = mapped_impl || sites.stack_site || !sites.fields.is_empty();
    let live_data_site = path.ends_with("/command/commands/data.rs") && !sites.nbt.is_empty();
    let has_site = sites.stack_site
        || !sites.fields.is_empty()
        || live_data_site
        || scoped && (!sites.mutators.is_empty() || !sites.nbt.is_empty() || !sites.boundaries.is_empty() || !sites.lifecycle.is_empty());
    if !has_site { return Vec::new(); }
    let mut out = Vec::new();
    let mut sink = RecordSink::new(path);
    for index in &sites.fields { sink.push_token(&mut out, &item, "direct-field-access", fingerprint, &body[*index]); }
    for (kind, indexes) in [("stock-mutator", &sites.mutators), ("nbt-component-site", &sites.nbt), ("boundary", &sites.boundaries), ("lifecycle-site", &sites.lifecycle)] {
        for index in indexes { sink.push_token(&mut out, &item, kind, fingerprint, &body[*index]); }
    }
    if sites.stack_site && out.is_empty() {
        sink.push(&mut out, Candidate { item: &item, kind: "itemstack-site", field: &span.name, line: line_for(tokens, span.start), fingerprint });
    }
    out
}

fn scan_type_fields(
    path: &str,
    tokens: &[Token<'_>],
    spans: &[Span],
    aliases: &BTreeSet<String>,
) -> Vec<Record> {
    let mut out = Vec::new();
    let mut sink = RecordSink::new(path);
    for span in spans.iter().filter(|span| span.kind == "type") {
        for (start, end) in field_segments(tokens, span.open, span.end) {
            let (name, type_start) = field_name(tokens, start, end);
            let callback_field = CALLBACK_FIELDS.contains(&name.as_str());
            if callback_field
                || span.name == "ItemStack"
                || is_stack_type(&tokens[type_start..end], aliases)
                || is_mapped_state_field(&name)
            {
                sink.push(
                    &mut out,
                    Candidate {
                        item: &span.name,
                        kind: if callback_field {
                            "boundary"
                        } else {
                            "field-declaration"
                        },
                        field: &name,
                        line: line_for(tokens, start),
                        fingerprint: &tokens[span.start..span.end],
                    },
                );
            }
        }
    }
    out
}

fn mapped_macro_names(
    tokens: &[Token<'_>],
    spans: &[Span],
    aliases: &BTreeSet<String>,
) -> BTreeSet<String> {
    spans
        .iter()
        .filter(|span| span.kind == "macro")
        .filter(|span| {
            tokens[span.start..span.end].iter().any(|token| {
                token.text == "ItemStack"
                    || aliases.contains(token.text)
                    || is_mapped_state_field(token.text)
            })
        })
        .map(|span| span.name.clone())
        .collect()
}

fn scan_macros(
    path: &str,
    tokens: &[Token<'_>],
    spans: &[Span],
    mapped_names: &BTreeSet<String>,
) -> Vec<Record> {
    let mut out = Vec::new();
    let mut sink = RecordSink::new(path);
    for span in spans.iter().filter(|span| span.kind == "macro") {
        if mapped_names.contains(&span.name) {
            let body = &tokens[span.start..span.end];
            sink.push(
                &mut out,
                Candidate {
                    item: &format!("macro_rules!{}", span.name),
                    kind: "macro-definition",
                    field: &span.name,
                    line: line_for(tokens, span.start),
                    fingerprint: body,
                },
            );
        }
    }
    for (index, token) in tokens.iter().enumerate() {
        if !mapped_names.contains(token.text)
            || tokens.get(index + 1).is_none_or(|next| next.text != "!")
            || spans
                .iter()
                .any(|span| span.kind == "macro" && span.start < index && span.end > index)
        {
            continue;
        }
        let parent = spans
            .iter()
            .filter(|span| span.kind != "macro" && span.start < index && span.end > index)
            .min_by_key(|span| span.end - span.start);
        let item = parent.map_or_else(|| "<module>".to_owned(), |span| enclosing(span, spans));
        let fingerprint = parent.map_or(tokens, |span| &tokens[span.start..span.end]);
        sink.push(
            &mut out,
            Candidate {
                item: &item,
                kind: "macro-invocation",
                field: token.text,
                line: token.line,
                fingerprint,
            },
        );
    }
    out
}

pub fn scan_file(path: &str, source: &str) -> Vec<Record> {
    let tokens = skip_test_modules(&tokenize(source));
    let spans = item_spans(&tokens);
    let aliases = stack_aliases(&tokens);
    let mapped_names = mapped_macro_names(&tokens, &spans, &aliases);
    scan_file_with_fields(path, &tokens, &mapped_names)
}

fn scan_file_with_fields(
    path: &str,
    tokens: &[Token<'_>],
    mapped_names: &BTreeSet<String>,
) -> Vec<Record> {
    let spans = item_spans(tokens);
    let aliases = stack_aliases(tokens);
    let mapped_types = mapped_type_names(tokens, &spans, &aliases);
    let fields: BTreeSet<_> = STATE_FIELDS
        .iter()
        .map(|field| (*field).to_owned())
        .collect();
    let mut out = scan_type_fields(path, tokens, &spans, &aliases);
    for span in spans.iter().filter(|span| span.kind == "function") {
        out.extend(scan_method(
            path,
            tokens,
            &spans,
            span,
            &fields,
            &aliases,
            &mapped_types,
        ));
    }
    out.extend(scan_macros(path, tokens, &spans, mapped_names));
    if !out.is_empty() {
        out.push(record(
            path,
            "<module>",
            "module-fingerprint",
            "<token-normalized-file>",
            1,
            1,
            tokens,
        ));
    }
    out
}

pub fn discover_workspace(root: &Path) -> Result<Vec<Record>, String> {
    let mut files = Vec::new();
    for workspace_dir in ["crates", "tools"] {
        let directory = root.join(workspace_dir);
        if !directory.exists() {
            continue;
        }
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                gather(&entry.path(), root, &mut files).map_err(|error| error.to_string())?;
            }
        }
    }
    files.sort();
    let mut mapped_names = BTreeSet::new();
    for relative in &files {
        let path = root.join(relative);
        let source =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let tokens = skip_test_modules(&tokenize(&source));
        let spans = item_spans(&tokens);
        let aliases = stack_aliases(&tokens);
        mapped_names.extend(mapped_macro_names(&tokens, &spans, &aliases));
    }
    let mut records = Vec::new();
    for relative in files {
        let path = root.join(&relative);
        let source =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        let tokens = skip_test_modules(&tokenize(&source));
        records.extend(scan_file_with_fields(&relative, &tokens, &mapped_names));
    }
    records.sort_by(|left, right| left.site_key.cmp(&right.site_key));
    Ok(records)
}
