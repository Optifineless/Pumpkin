use super::{Imports, Token, imported_lock, leaf, lock_kind, path_at, path_is};

fn resolve_once(imports: &Imports, path: &str) -> Vec<String> {
    let (head, tail) = path.split_once("::").unwrap_or((path, ""));
    imports.paths.get(head).map_or_else(
        || vec![path.into()],
        |paths| {
            paths
                .iter()
                .map(|p| {
                    if tail.is_empty() {
                        p.clone()
                    } else {
                        format!("{p}::{tail}")
                    }
                })
                .collect()
        },
    )
}

fn resolve(imports: &Imports, path: &str) -> Vec<String> {
    let mut paths: Vec<String> = vec![path.into()];
    for _ in 0..imports.paths.len() {
        let mut next: Vec<_> = paths
            .iter()
            .flat_map(|p| resolve_once(imports, p))
            .collect();
        next.sort();
        next.dedup();
        if next == paths {
            break;
        }
        paths = next;
    }
    paths
}

fn type_path(t: &[Token<'_>], start: usize) -> String {
    let mut end = start;
    while t.get(end + 1).is_some_and(|x| x.text == "::")
        && t.get(end + 2).is_some_and(|x| x.text != "<")
    {
        end += 2;
    }
    t.get(start).map_or_else(String::new, |_| path_at(t, end))
}

pub fn aliases(t: &[Token<'_>], imports: &mut Imports) {
    let resolved: Vec<_> = imports
        .locks
        .keys()
        .map(|name| {
            let kinds: Vec<_> = resolve(imports, name)
                .iter()
                .filter_map(|p| lock_kind(p))
                .collect();
            (name.clone(), kinds)
        })
        .collect();
    for (name, kinds) in resolved {
        if !kinds.is_empty() {
            imports.locks.insert(name, kinds);
        }
    }
    let mut pending = Vec::new();
    for i in 0..t.len() {
        if t[i].text != "type" {
            continue;
        }
        let Some(name) = t.get(i + 1) else {
            continue;
        };
        let Some(eq) = (i + 2..t.len())
            .take_while(|j| t[*j].text != ";")
            .find(|j| t[*j].text == "=")
        else {
            continue;
        };
        pending.push((name.text, type_path(t, eq + 1)));
    }
    // Iterate to resolve forward references and chains, without following cycles forever.
    for _ in 0..pending.len() {
        for (name, target) in &pending {
            let paths = resolve(imports, target);
            let kinds: Vec<_> = paths.iter().filter_map(|p| lock_kind(p)).collect();
            if !kinds.is_empty() {
                let known = imports.locks.entry((*name).into()).or_default();
                known.extend(kinds);
                known.sort();
                known.dedup();
                let known = imports.paths.entry((*name).into()).or_default();
                known.extend(paths);
                known.sort();
                known.dedup();
            }
        }
    }
}

fn method(t: &[Token<'_>], i: usize) -> Option<usize> {
    if t.get(i + 1)?.text != "::" {
        return None;
    }
    let mut next = i + 2;
    if t.get(next)?.text == "<" {
        let mut depth = 0usize;
        while next < t.len() {
            match t[next].text {
                "<" => depth += 1,
                ">" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            next += 1;
        }
        if t.get(next + 1)?.text != "::" {
            return None;
        }
        next += 2;
    }
    matches!(t.get(next)?.text, "new" | "default" | "const_new")
        .then_some(next)
        .filter(|j| t.get(j + 1).is_some_and(|x| matches!(x.text, "(" | "::")))
}

fn typed_default(t: &[Token<'_>], i: usize, imports: &Imports) -> Option<String> {
    let start = (0..i)
        .rev()
        .find(|j| matches!(t[*j].text, ";" | "{" | "}"))
        .map_or(0, |j| j + 1);
    let eq = (start..i).rev().find(|j| t[*j].text == "=");
    let colon = (start..eq.unwrap_or(i)).rev().find(|j| t[*j].text == ":")?;
    let end = eq.unwrap_or(i);
    let mut type_start = colon + 1;
    // Struct initializers may name a field whose declaration is elsewhere in this file.
    let type_end = if eq.is_none() {
        let field = t.get(colon.checked_sub(1)?)?.text;
        let declaration = (0..t.len()).find(|j| {
            *j != colon.saturating_sub(1)
                && t[*j].text == field
                && t.get(j + 1).is_some_and(|x| x.text == ":")
                && t.get(j + 2).is_some_and(|x| x.text != "Default")
        })?;
        type_start = declaration + 2;
        (type_start..t.len())
            .find(|j| matches!(t[*j].text, "," | "}" | "="))
            .unwrap_or(t.len())
    } else {
        end
    };
    let path = type_path(t, type_start);
    let kinds: Vec<_> = resolve(imports, &path)
        .iter()
        .filter_map(|p| lock_kind(p))
        .collect();
    if kinds.len() == 1 {
        if kinds[0].starts_with("unknown::") {
            return Some("unknown::Default::default".into());
        }
        return kinds[0]
            .strip_suffix("::new")
            .map(|p| format!("{p}::default"));
    }
    if imports.locks.contains_key(&path) {
        return Some(imported_lock(imports, &path).replace("::new", "::default"));
    }
    t[type_start..type_end]
        .iter()
        .any(|x| x.text.contains("Mutex") || x.text.contains("RwLock"))
        .then(|| "unknown::Default::default".into())
}

pub fn constructor(t: &[Token<'_>], i: usize, imports: &Imports) -> Option<String> {
    if t.get(i + 1).is_none_or(|x| !matches!(x.text, "::" | "!")) {
        return None;
    }
    if leaf(t[i].text).is_none()
        && t[i].text != "Default"
        && !imports.locks.contains_key(t[i].text)
        && t.get(i + 1).is_none_or(|x| x.text != "!")
    {
        return None;
    }
    let full = path_at(t, i);
    if t.get(i + 1).is_some_and(|x| x.text == "!") {
        let paths = resolve(imports, &full);
        for (macro_name, lock) in [("const_mutex", "Mutex"), ("const_rwlock", "RwLock")] {
            if paths.contains(&format!("parking_lot::{macro_name}")) {
                return Some(format!("parking_lot::{lock}::{macro_name}!"));
            }
        }
    }
    let m = method(t, i)?;
    if t[i].text == "Default" && t[m].text == "default" {
        return typed_default(t, i, imports);
    }
    let paths = resolve(imports, &full);
    let mut kinds: Vec<_> = paths.iter().filter_map(|p| lock_kind(p)).collect();
    if full.contains("::") {
        kinds.sort();
        kinds.dedup();
        if kinds.len() != 1 {
            return None;
        }
        return Some(kinds[0].replace("::new", &format!("::{}", t[m].text)));
    }
    if leaf(t[i].text).is_some() || imports.locks.contains_key(t[i].text) {
        return Some(
            imported_lock(imports, t[i].text).replace("::new", &format!("::{}", t[m].text)),
        );
    }
    None
}

pub fn bridge(t: &[Token<'_>], i: usize, imports: &Imports) -> Option<(String, bool)> {
    if t.get(i + 1).is_none_or(|x| x.text != "(")
        || i.checked_sub(1).is_some_and(|j| t[j].text == "fn")
    {
        return None;
    }
    if !matches!(
        t[i].text,
        "block_on" | "block_in_place" | "spawn_blocking" | "spawn"
    ) && !imports.paths.get(t[i].text).is_some_and(|paths| {
        paths.iter().any(|p| {
            matches!(
                p.as_str(),
                "futures::executor::block_on" | "tokio::task::block_in_place"
            )
        })
    }) {
        return None;
    }
    let full = path_at(t, i);
    let paths = resolve(imports, &full);
    if paths.iter().any(|p| p == "futures::executor::block_on") {
        return Some(("futures::executor::block_on".into(), true));
    }
    if paths.iter().any(|p| p == "tokio::task::block_in_place") || t[i].text == "block_in_place" {
        return Some(("block_in_place".into(), true));
    }
    if t[i].text == "block_on" {
        return Some(("block_on".into(), true));
    }
    if t[i].text == "spawn_blocking" {
        return Some(("spawn_blocking".into(), false));
    }
    if t[i].text == "spawn" && path_is(t, i, "rayon::spawn") {
        return Some(("rayon::spawn".into(), false));
    }
    None
}
