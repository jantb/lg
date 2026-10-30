//! The paths a patch names: read off its headers, with git's quoting undone,
//! and quoted again for git.

/// The old and new paths a `diff --git` line names, without their `a/` and
/// `b/` prefixes.
///
/// Git quotes a path holding a quote, a backslash, a control character or
/// (by default) anything outside ASCII, and leaves one with spaces bare. A
/// bare pair is split where both halves name the same file, which is the case
/// for everything but a rename; the `rename from`/`rename to` lines settle
/// those, and [`parse_patch`] reads them.
pub fn diff_git_paths(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    let (old, new) = split_header_paths(rest)?;
    Some((strip_prefix_owned(old, "a/"), strip_prefix_owned(new, "b/")))
}

fn split_header_paths(rest: &str) -> Option<(String, String)> {
    if rest.starts_with('"') {
        let (old, len) = unquote(rest)?;
        let new = rest[len..].strip_prefix(' ')?;
        return Some((old, header_path(new)));
    }
    if rest.ends_with('"')
        && let Some(at) = rest.find(" \"")
        && let Some((new, len)) = unquote(&rest[at + 1..])
        && at + 1 + len == rest.len()
    {
        return Some((rest[..at].to_string(), new));
    }
    for (at, _) in rest.match_indices(' ') {
        let (old, new) = (&rest[..at], &rest[at + 1..]);
        if old
            .strip_prefix("a/")
            .is_some_and(|old| Some(old) == new.strip_prefix("b/"))
        {
            return Some((old.to_string(), new.to_string()));
        }
    }
    if let Some(at) = rest.find(" b/") {
        return Some((rest[..at].to_string(), rest[at + 1..].to_string()));
    }
    let (old, new) = rest.split_once(' ')?;
    Some((old.to_string(), new.to_string()))
}

/// A path as a header line ends with it: quoted, or bare.
pub(super) fn header_path(text: &str) -> String {
    match unquote(text) {
        Some((path, _)) if text.starts_with('"') => path,
        _ => text.to_string(),
    }
}

fn strip_prefix_owned(path: String, prefix: &str) -> String {
    match path.strip_prefix(prefix) {
        Some(rest) => rest.to_string(),
        None => path,
    }
}

/// The path of a `--- a/x` or `+++ b/x` line, `None` for `/dev/null`. Git
/// ends the name with a tab when it has spaces in it, and `diff -u` puts a
/// timestamp after that tab.
pub(super) fn marker_path(rest: &str, prefix: &str) -> Option<String> {
    let path = if rest.starts_with('"') {
        unquote(rest)?.0
    } else {
        rest.split('\t').next().unwrap_or(rest).to_string()
    };
    (path != "/dev/null").then(|| strip_prefix_owned(path, prefix))
}

/// A C-style quoted string at the start of `text`: what it says, and how many
/// bytes of `text` it took up, quotes included.
fn unquote(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), i + 1)),
            b'\\' => {
                let escaped = *bytes.get(i + 1)?;
                i += 2;
                match escaped {
                    b'0'..=b'7' => {
                        let mut value = u32::from(escaped - b'0');
                        for _ in 0..2 {
                            match bytes.get(i) {
                                Some(digit @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(digit - b'0');
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(u8::try_from(value).ok()?);
                    }
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'v' => out.push(0x0b),
                    other => out.push(other),
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    None
}

/// `prefix` and `path`, quoted the way git would need to read them back when
/// the path holds a quote, a backslash or a control character.
pub(super) fn quote_path(prefix: &str, path: &str) -> String {
    let full = format!("{prefix}{path}");
    if !full
        .chars()
        .any(|ch| ch == '"' || ch == '\\' || ch.is_control())
    {
        return full;
    }
    let mut out = String::from("\"");
    for ch in full.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ch if ch.is_control() => {
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("\\{byte:03o}"));
                }
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
