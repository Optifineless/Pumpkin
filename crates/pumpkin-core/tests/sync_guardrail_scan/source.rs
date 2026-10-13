#[derive(Clone)]
pub struct Token<'a> {
    pub text: &'a str,
    pub line: usize,
    pub literal_before: Option<&'a str>,
}

pub fn raw_end(b: &[u8], i: usize) -> Option<usize> {
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
pub fn tokenize(s: &str) -> Vec<Token<'_>> {
    let b = s.as_bytes();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    let (mut literal_start, mut literal_end) = (None, None);
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
        } else if let Some(end) = raw_end(b, i) { literal_start.get_or_insert(i); literal_end = Some(end); line += s[i..end].bytes().filter(|x| *x == b'\n').count(); i = end; }
        else if b[i] == b'"' || (matches!(b[i], b'b' | b'c') && b.get(i + 1) == Some(&b'"')) {
            let quote = i + usize::from(b[i] != b'"'); let mut j = quote + 1;
            while j < b.len() && b[j] != b'"' { if b[j] == b'\\' { j += 1; } j += 1; }
            let end = (j + 1).min(b.len()); literal_start.get_or_insert(i); literal_end = Some(end);
            line += s[quote..end].bytes().filter(|x| *x == b'\n').count(); i = end;
        } else if b[i].is_ascii_alphanumeric() || b[i] == b'_' {
            let start = i; i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            out.push(Token { text: &s[start..i], line, literal_before: literal_start.take().zip(literal_end.take()).map(|(from, to)| &s[from..to]) });
        } else if b[i..].starts_with(b"::") { out.push(Token { text: "::", line, literal_before: literal_start.take().zip(literal_end.take()).map(|(from, to)| &s[from..to]) }); i += 2; }
        else { let start = i; let ch = s[i..].chars().next().unwrap_or(' '); i += ch.len_utf8(); out.push(Token { text: &s[start..i], line, literal_before: literal_start.take().zip(literal_end.take()).map(|(from, to)| &s[from..to]) }); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::tokenize;

    #[test]
    fn string_literals_keep_fingerprints_without_entering_the_token_stream() {
        let tokens =
            tokenize(r###"const TEXT: &str = "first"; const RAW: &str = r##"second"##;"###);
        let text: Vec<_> = tokens.iter().map(|token| token.text).collect();
        assert_eq!(
            text,
            [
                "const", "TEXT", ":", "&", "str", "=", ";", "const", "RAW", ":", "&", "str", "=",
                ";"
            ]
        );
        let literals: Vec<_> = tokens
            .iter()
            .filter_map(|token| token.literal_before)
            .collect();
        assert_eq!(literals, [r#""first""#, r###"r##"second"##"###]);
    }
}
