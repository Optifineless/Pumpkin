#[path = "sync_guardrail_scan/coverage.rs"]
mod coverage;
#[path = "sync_guardrail_scan/fixtures.rs"]
mod fixtures;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Token<'a> {
    pub text: &'a str,
    pub line: usize,
}
#[derive(Clone)]
struct Site {
    key: String,
    kind: String,
    path: String,
    line: usize,
    enforced: bool,
}
#[derive(Default)]
pub struct Imports {
    locks: HashMap<String, Vec<String>>,
    pub paths: HashMap<String, Vec<String>>,
    globs: Vec<String>,
}

fn raw_end(b: &[u8], i: usize) -> Option<usize> {
    let mut r = i + usize::from(b.get(i) == Some(&b'b'));
    if b.get(r) != Some(&b'r') {
        return None;
    }
    r += 1;
    let mut quote = r;
    while b.get(quote) == Some(&b'#') {
        quote += 1;
    }
    if b.get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - r;
    let mut j = quote + 1;
    while j < b.len() {
        if b[j] == b'"'
            && b.get(j + 1..j + 1 + hashes)
                .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
        {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(b.len())
}

#[rustfmt::skip]
#[must_use]
pub fn tokenize(s: &str) -> Vec<Token<'_>> {
    let b = s.as_bytes();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < b.len() {
        if b[i].is_ascii_whitespace() { line += usize::from(b[i] == b'\n'); i += 1; }
        else if b[i..].starts_with(b"//") { while i < b.len() && b[i] != b'\n' { i += 1; } }
        else if b[i..].starts_with(b"/*") {
            i += 2; let mut depth = 1;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") { depth += 1; i += 2; }
                else if b[i..].starts_with(b"*/") { depth -= 1; i += 2; }
                else { line += usize::from(b[i] == b'\n'); i += 1; }
            }
        } else if let Some(end) = raw_end(b, i) { line += s[i..end].bytes().filter(|x| *x == b'\n').count(); i = end; }
        else if b[i] == b'"' || (matches!(b[i], b'b' | b'c') && b.get(i + 1) == Some(&b'"')) {
            let quote = i + usize::from(b[i] != b'"'); let mut j = quote + 1;
            while j < b.len() && b[j] != b'"' { if b[j] == b'\\' { j += 1; } j += 1; }
            i = (j + 1).min(b.len()); line += s[quote..i].bytes().filter(|x| *x == b'\n').count();
        } else if b[i].is_ascii_alphanumeric() || b[i] == b'_' {
            let start = i; i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            out.push(Token { text: &s[start..i], line });
        } else if b[i..].starts_with(b"::") { out.push(Token { text: "::", line }); i += 2; }
        else { let ch = s[i..].chars().next().unwrap_or(' '); out.push(Token { text: &s[i..i + ch.len_utf8()], line }); i += ch.len_utf8(); }
    }
    out
}

fn leaf(path: &str) -> Option<&str> {
    let name = path.rsplit("::").next()?;
    [
        "Mutex",
        "RwLock",
        "ReentrantMutex",
        "FairMutex",
        "ScreenMutex",
    ]
    .contains(&name)
    .then_some(name)
}
fn lock_kind(path: &str) -> Option<String> {
    let name = leaf(path)?;
    if name == "ScreenMutex" {
        return Some("ScreenMutex::new".into());
    }
    if path == format!("std::sync::{name}")
        || path == format!("tokio::sync::{name}")
        || path == format!("pumpkin_util::sync::{name}")
    {
        Some(format!("{path}::new"))
    } else if path.starts_with("parking_lot::") {
        Some(format!("parking_lot::{name}::new"))
    } else {
        Some(format!("unknown::{name}::new"))
    }
}
fn add_import(out: &mut Imports, path: &str, alias: Option<&str>) {
    let Some(name) = path.rsplit("::").next() else {
        return;
    };
    out.paths
        .entry(alias.unwrap_or(name).into())
        .or_default()
        .push(path.into());
    if let Some(kind) = lock_kind(path) {
        out.locks
            .entry(alias.unwrap_or(name).into())
            .or_default()
            .push(kind);
    }
}
#[rustfmt::skip]
fn use_tree(t: &[Token<'_>], i: &mut usize, prefix: String, out: &mut Imports) {
    let mut path = prefix;
    while *i < t.len() {
        match t[*i].text {
            "{" => {
                *i += 1; while *i < t.len() && t[*i].text != "}" {
                    use_tree(t, i, path.clone(), out);
                    if t.get(*i).is_some_and(|x| x.text != "}") { *i += 1; }
                }
                *i += usize::from(*i < t.len()); return;
            }
            "::" => {
                *i += 1; if t.get(*i).is_some_and(|x| x.text == "{") { continue; }
                if t.get(*i).is_some_and(|x| x.text == "*") { out.globs.push(path); *i += 1; return; }
            }
            "as" => {
                *i += 1; let alias = t.get(*i).map(|x| x.text.to_owned()); *i += usize::from(alias.is_some());
                add_import(out, &path, alias.as_deref().filter(|x| *x != "_")); return;
            }
            "," | "}" => { add_import(out, &path, None); return; }
            _ => {
                if !path.is_empty() { path.push_str("::"); } path.push_str(t[*i].text); *i += 1;
            }
        }
    }
    add_import(out, &path, None);
}
#[rustfmt::skip]
#[must_use]
pub fn imports(t: &[Token<'_>]) -> Imports {
    let mut out = Imports::default(); let mut i = 0;
    while i < t.len() { if t[i].text == "use" { let mut end = i + 1; while end < t.len() && t[end].text != ";" { end += 1; }
        let mut cursor = i + 1; use_tree(&t[..end], &mut cursor, String::new(), &mut out); i = end; } i += 1; } out
}

#[rustfmt::skip]
#[must_use]
pub fn owners(t: &[Token<'_>]) -> (Vec<usize>, Vec<String>) {
    let mut out = vec![0; t.len()]; let mut names = vec!["<module>".into()]; let mut scopes = Vec::<(usize, usize)>::new();
    let (mut depth, mut current, mut func) = (0usize, 0usize, None);
    for i in 0..t.len() {
        let word = t[i].text;
        if word == "}" { scopes.retain(|(level, _)| *level != depth); current = scopes.last().map_or(0, |(_, id)| *id); depth = depth.saturating_sub(1); continue; }
        out[i] = current;
        if word == "{" {
            let name = if func.as_ref().is_some_and(|(level, _)| *level == depth) { func.take().map(|(_, name)| name) } else { None };
            depth += 1; if let Some(name) = name { let id = names.len(); names.push(name); scopes.push((depth, id)); current = id; }
        } else if word == "fn" && t.get(i + 1).is_some_and(|x| x.text.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')) { func = Some((depth, t[i + 1].text.to_owned())); }
        else if word == ";" && func.as_ref().is_some_and(|(level, _)| *level == depth) { func = None; }
    }
    (out, names)
}
#[must_use]
pub fn path_at(t: &[Token<'_>], i: usize) -> String {
    let mut start = i;
    while start >= 2
        && t[start - 1].text == "::"
        && t[start - 2]
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        start -= 2;
    }
    t[start..=i]
        .iter()
        .filter(|x| x.text != "::")
        .map(|x| x.text)
        .collect::<Vec<_>>()
        .join("::")
}
#[rustfmt::skip]
fn path_is(t: &[Token<'_>], i: usize, path: &str) -> bool {
    let mut cursor = i;
    let parts = path.rsplit("::").count();
    for (part_index, part) in path.rsplit("::").enumerate() {
        if t.get(cursor).is_none_or(|token| token.text != part) { return false; }
        if part_index + 1 < parts {
            if cursor < 2 || t[cursor - 1].text != "::" { return false; }
            cursor -= 2;
        }
    }
    cursor == 0 || t[cursor - 1].text != "::" || cursor == 1
}
fn imported_lock(imports: &Imports, name: &str) -> String {
    if name == "ScreenMutex" {
        return "ScreenMutex::new".into();
    }
    let mut kinds = imports.locks.get(name).cloned().unwrap_or_default();
    for glob in &imports.globs {
        let path = match glob.as_str() {
            "std::sync" => format!("std::sync::{name}"),
            "tokio::sync" => format!("tokio::sync::{name}"),
            "parking_lot" => format!("parking_lot::{name}"),
            "pumpkin_util::sync" => format!("pumpkin_util::sync::{name}"),
            _ => continue,
        };
        if let Some(kind) = lock_kind(&path) {
            kinds.push(kind);
        }
    }
    kinds.sort();
    kinds.dedup();
    if kinds.len() == 1 {
        return kinds.remove(0);
    }
    let lock = kinds
        .first()
        .and_then(|kind| kind.strip_suffix("::new"))
        .and_then(|path| path.rsplit("::").next())
        .unwrap_or(name);
    format!("unknown::{lock}::new")
}
#[rustfmt::skip]
fn scan_source(path: &str, source: &str) -> Vec<Site> {
    let t = tokenize(source); let mut imports = imports(&t); coverage::aliases(&t, &mut imports); let (owners, names) = owners(&t);
    let mut found = Vec::<(usize, String, bool)>::new();
    for i in 0..t.len() {
        if let Some(kind) = coverage::constructor(&t, i, &imports) {
            let enforced = !kind.starts_with("pumpkin_util::sync::") && !kind.contains("ScreenMutex");
            found.push((i, kind, enforced));
        }
        if let Some((kind, enforced)) = coverage::bridge(&t, i, &imports) {
            found.push((i, kind, enforced));
        }
    }
    found.sort_by_key(|(i, _, _)| *i);
    let mut ordinal = HashMap::<(String, String), usize>::new();
    found.into_iter().map(|(i, kind, enforced)| {
        let owner = owners.get(i).and_then(|id| names.get(*id)).map_or("<module>", String::as_str); let n = ordinal.entry((owner.into(), kind.clone())).or_default(); *n += 1;
        Site { key: format!("{path}::{owner}::{kind}#{}", *n), kind, path: path.into(), line: t[i].line, enforced }
    }).collect()
}

#[rustfmt::skip]
fn walk(dir: &Path, root: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?; let path = item.path();
        if item.file_type()?.is_dir() {
            if item.file_name() != "tests" { walk(&path, root, files)?; }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.strip_prefix(root).unwrap_or(&path).into());
        }
    }
    Ok(())
}
#[rustfmt::skip]
fn scan_workspace(root: &Path) -> std::io::Result<Vec<Site>> {
    let mut files = Vec::new();
    for item in fs::read_dir(root.join("crates"))? { let src = item?.path().join("src"); if src.is_dir() { walk(&src, root, &mut files)?; } }
    files.sort(); let mut sites = Vec::new();
    for relative in files {
        let file = root.join(&relative); let source = fs::read_to_string(&file)?;
        if ["Mutex", "RwLock", "const_mutex", "const_rwlock", "block_on", "block_in_place", "spawn_blocking", "rayon::spawn"].iter().any(|key| source.contains(key)) {
            sites.extend(scan_source(&relative.to_string_lossy().replace('\\', "/"), &source));
        }
    }
    Ok(sites)
}
fn diff(actual: &[Site], contents: &str) -> (Vec<String>, Vec<String>) {
    let actual: BTreeSet<_> = actual.iter().map(|site| site.key.as_str()).collect();
    let allow: BTreeSet<_> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    (
        actual.difference(&allow).map(|key| (*key).into()).collect(),
        allow.difference(&actual).map(|key| (*key).into()).collect(),
    )
}
fn write_allowlist(path: &Path, sites: &[Site]) -> std::io::Result<()> {
    let mut text = String::from(
        "# Reviewed baseline of raw synchronization and blocking bridge sites.\n# New entries require a justification comment; see rules B1 and B3.\n",
    );
    let mut keys: Vec<_> = sites
        .iter()
        .filter(|site| site.enforced)
        .map(|site| site.key.as_str())
        .collect();
    keys.sort_unstable();
    for key in keys {
        text.push_str(key);
        text.push('\n');
    }
    fs::write(path, text)
}

#[test]
#[expect(clippy::print_stderr, reason = "the guardrail prints baseline counts for the report")]
#[rustfmt::skip]
fn synchronization_sites_are_allowlisted() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(Path::parent);
    assert!(root.is_some(), "could not locate workspace root");
    let started = std::time::Instant::now();
    let scanned = scan_workspace(root.unwrap_or(&manifest));
    eprintln!("sync guardrail scan runtime: {:?}", started.elapsed());
    assert!(scanned.is_ok(), "could not scan workspace source: {:?}", scanned.as_ref().err());
    let sites = scanned.unwrap_or_default();
    let allow_path = manifest.join("tests/sync_guardrail_allowlist.txt");
    let loaded = fs::read_to_string(&allow_path);
    assert!(loaded.is_ok(), "could not read {}", allow_path.display());
    let enforced: Vec<_> = sites.iter().filter(|site| site.enforced).cloned().collect();
    let (new, stale) = diff(&enforced, &loaded.unwrap_or_default());
    eprintln!("sync guardrail mismatch summary: new={}, stale={}", new.len(), stale.len());
    if env::var("SYNC_GUARDRAIL_WRITE").as_deref() == Ok("1") { assert!(write_allowlist(&allow_path, &sites).is_ok(), "could not write allowlist"); }
    let mut counts = BTreeMap::<String, usize>::new();
    for kind in ["std::sync::Mutex::new", "std::sync::RwLock::new", "tokio::sync::Mutex::new", "tokio::sync::RwLock::new", "parking_lot::Mutex::new", "parking_lot::RwLock::new", "parking_lot::ReentrantMutex::new", "parking_lot::FairMutex::new", "unknown::Mutex::new", "unknown::RwLock::new", "block_on", "futures::executor::block_on", "block_in_place", "rayon::spawn", "spawn_blocking"] { counts.insert(kind.into(), 0); }
    for site in &sites { *counts.entry(site.kind.clone()).or_default() += 1; }
    let enforced_counts = counts.iter().filter(|(kind, _)| !matches!(kind.as_str(), "rayon::spawn" | "spawn_blocking") && !kind.starts_with("pumpkin_util::sync::") && !kind.contains("ScreenMutex")).map(|(kind, n)| format!("{kind}={n}")).collect::<Vec<_>>().join(", ");
    eprintln!("sync guardrail enforced counts: {enforced_counts}");
    let informational_counts = counts.iter().filter(|(kind, _)| matches!(kind.as_str(), "rayon::spawn" | "spawn_blocking") || kind.starts_with("pumpkin_util::sync::") || kind.contains("ScreenMutex")).map(|(kind, n)| format!("{kind}={n}")).collect::<Vec<_>>().join(", ");
    eprintln!("sync guardrail informational counts: {informational_counts}");
    let informational = sites.iter().filter(|site| !site.enforced).map(|site| format!("{}:{} {}", site.path, site.line, site.key)).collect::<Vec<_>>().join("\n");
    eprintln!("informational only sites (not enforced):\n{informational}");
    let mut files = HashMap::<&str, usize>::new();
    for site in &sites { *files.entry(&site.path).or_default() += 1; }
    let mut top: Vec<_> = files.into_iter().collect(); top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    eprintln!("top synchronization files: {}", top.iter().take(10).map(|(path, count)| format!("{path}={count}")).collect::<Vec<_>>().join(", "));
    let location = |key: &str| sites.iter().find(|site| site.key == key).map_or_else(|| key.into(), |site| format!("{}:{}: {key}", site.path, site.line));
    assert!(env::var("SYNC_GUARDRAIL_WRITE").as_deref() == Ok("1") || (new.is_empty() && stale.is_empty()),
        "sync guardrail mismatch (rules B1/B3 in rules-regression-prevention.md).\nNew sites:\n{}\nStale allowlist entries:\n{}\nReview each site: use the instrumented lock wrapper from PR #149 once merged, or add the key here with a justification comment.\nRegenerate the baseline with: SYNC_GUARDRAIL_WRITE=1 cargo test -p pumpkin-core --test sync_guardrail_scan -- --nocapture",
        if new.is_empty() { "  (none)".into() } else { new.iter().map(|key| format!("  {}", location(key))).collect::<Vec<_>>().join("\n") },
        if stale.is_empty() { "  (none)".into() } else { stale.iter().map(|key| format!("  {key}")).collect::<Vec<_>>().join("\n") });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skips_comments_and_strings() {
        let src = r####"// std::sync::Mutex::new(())
            /* .block_on(()) */ const S: &str = r###"tokio::sync::RwLock::new(())"###;
            fn actual() { std::sync::Mutex::new(()); }"####;
        let sites = scan_source("src/a.rs", src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].kind, "std::sync::Mutex::new");
    }
    #[test]
    fn resolves_import_aliases_and_ambiguity() {
        let direct = scan_source(
            "src/a.rs",
            "use std::sync::Mutex; fn make() { Mutex::new(()); }",
        );
        assert_eq!(direct[0].kind, "std::sync::Mutex::new");
        let src = "use std::sync::Mutex as Lock; use tokio::sync::Mutex as Lock; fn make() { Lock::new(()); }";
        assert_eq!(scan_source("src/a.rs", src)[0].kind, "unknown::Mutex::new");
    }
    #[test]
    fn names_the_enclosing_function() {
        let sites = scan_source(
            "src/a.rs",
            "impl Widget { fn update(&self) { Mutex::new(()); } }",
        );
        assert_eq!(sites[0].key, "src/a.rs::update::unknown::Mutex::new#1");
        let macro_site = scan_source(
            "src/a.rs",
            "fn maker() { quote! { fn #name() { value.block_on(); } } }",
        );
        assert_eq!(macro_site[0].key, "src/a.rs::maker::block_on#1");
    }
    #[test]
    fn ignores_bridge_function_declarations() {
        let sites = scan_source(
            "src/a.rs",
            "fn spawn_blocking() {} fn run() { spawn_blocking(); }",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].key, "src/a.rs::run::spawn_blocking#1");
    }
    #[test]
    fn detects_stale_allowlist_entries() {
        let actual = scan_source("src/a.rs", "fn current() { std::sync::Mutex::new(()); }");
        let (new, stale) = diff(&actual, "src/old.rs::gone::std::sync::Mutex::new#1\n");
        assert_eq!(new, vec!["src/a.rs::current::std::sync::Mutex::new#1"]);
        assert_eq!(stale, vec!["src/old.rs::gone::std::sync::Mutex::new#1"]);
    }
}
