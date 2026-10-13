use super::syntax::{imports, owners, path_at, tokenize};

const WRITES: &[&str] = &[
    "store",
    "swap",
    "compare_exchange",
    "compare_and_swap",
    "fetch_add",
    "fetch_sub",
    "rcu",
    "insert",
    "remove",
    "remove_if",
    "take",
    "push",
    "push_back",
    "clear",
    "get_mut",
    "write",
    "set_stack",
    "set_block_state",
    "add_block_entity_nbt",
];
fn api(name: &str) -> bool {
    name.contains("nbt")
        || [
            "serialize",
            "deserialize",
            "write_",
            "put_",
            "read_nbt",
            "from_nbt",
            "set_",
            "add_",
            "remove_",
            "update_",
            "apply_",
            "capture",
            "save",
            "load",
            "teleport",
            "spawn",
            "get_inventory",
            "get_stack",
            "get_mut",
            "clone_dyn",
            "write_data",
            "assert_not_in_snapshot",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

// Reuses #149's comment/string-safe lexer and enclosing-symbol inventory.
// This is a lexical alarm, not Rust type/alias analysis. Fingerprints include literal-bearing
// source lines at mapped sites; wrapping/comment edits there conservatively require review.
#[must_use]
pub fn sites(path: &str, source: &str) -> Vec<String> {
    let tokens = tokenize(source);
    let imports = imports(&tokens);
    let (owner_ids, names) = owners(&tokens);
    let lines: Vec<_> = source.lines().collect();
    let mut sites = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let called = tokens
            .get(i + 1)
            .is_some_and(|t| matches!(t.text, "(" | "!" | "::"));
        let declaration = i > 0 && tokens[i - 1].text == "fn";
        let mut resolved = token.text;
        for _ in 0..imports.paths.len() {
            let Some(path) = imports.paths.get(resolved).and_then(|paths| paths.first()) else {
                break;
            };
            let next = path.rsplit("::").next().unwrap_or(resolved);
            if next == resolved {
                break;
            }
            resolved = next;
        }
        let public_field = token.text == "pub" && tokens.get(i + 2).is_some_and(|t| t.text == ":");
        let assignment = token.text == "="
            && tokens
                .get(i + 1)
                .is_none_or(|t| !matches!(t.text, "=" | ">"))
            && i > 0
            && !matches!(tokens[i - 1].text, "=" | "!" | "<" | ">");
        let family = if public_field || declaration || (called && api(resolved)) {
            "capture/live-NBT/mutable-api"
        } else if assignment || (called && WRITES.contains(&resolved)) {
            "mapped-write"
        } else if called
            && (resolved.contains("chest") || resolved.contains("furnace"))
            && tokens.get(i + 1).is_some_and(|t| t.text == "!")
        {
            "macro-family"
        } else {
            continue;
        };
        let owner = owner_ids
            .get(i)
            .and_then(|id| names.get(*id))
            .map_or("<module>", String::as_str);
        let end = if public_field {
            tokens.iter().skip(i).find(|t| t.text == ",")
        } else if declaration {
            let (mut depth, mut groups) = (0usize, 0usize);
            tokens.iter().skip(i).find(|t| match t.text {
                "(" | "[" => {
                    groups += 1;
                    false
                }
                ")" | "]" => {
                    groups = groups.saturating_sub(1);
                    false
                }
                "{" => {
                    depth += 1;
                    false
                }
                "}" => {
                    depth = depth.saturating_sub(1);
                    depth == 0 && groups == 0
                }
                ";" => depth == 0 && groups == 0,
                _ => false,
            })
        } else {
            tokens.iter().skip(i).find(|t| t.text == ";")
        };
        let end_line = end.map_or(token.line, |t| t.line);
        let source_site = lines[token.line - 1..end_line].join("\n");
        let fingerprint = source_site.split_whitespace().collect::<Vec<_>>().join(" ");
        sites.push(format!(
            "{path}::{owner}::{family}::{}::{fingerprint}",
            path_at(&tokens, i)
        ));
    }
    sites
}

#[must_use]
pub fn boundaries(path: &str, source: &str) -> Vec<(String, String)> {
    let tokens = tokenize(source);
    let (owner_ids, names) = owners(&tokens);
    let lines: Vec<_> = source.lines().collect();
    tokens
        .iter()
        .enumerate()
        .filter_map(|(i, token)| {
            let name = path_at(&tokens, i);
            if !matches!(
                name.as_str(),
                "SnapshotScope::legacy"
                    | "assert_not_in_snapshot"
                    | "pumpkin_util::sync::assert_not_in_snapshot"
                    | "SnapshotCall::enter"
                    | "pumpkin_util::sync::SnapshotCall::enter"
            ) || tokens.get(i + 1).is_none_or(|t| t.text != "(")
            {
                return None;
            }
            let end = tokens.iter().skip(i).find(|t| t.text == ";")?;
            let call = lines[token.line - 1..end.line].join(" ");
            let boundary = if name == "SnapshotScope::legacy" {
                owner_ids.get(i).and_then(|id| names.get(*id))?.as_str()
            } else {
                call.split('"').nth(1)?
            };
            Some((
                format!("boundary:{path}:{}::{boundary}", token.line),
                call.split_whitespace().collect::<Vec<_>>().join(" "),
            ))
        })
        .collect()
}
