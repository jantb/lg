//! Syntax colouring for a line of code, shared by the diff, the source view
//! and the code blocks of rendered Markdown.

use std::borrow::Cow;

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

use crate::language::Language;

/// The keyword table the code lexer colours a language by. Languages without
/// one are drawn plain.
#[derive(Clone, Copy)]
enum Keywords {
    CSharp,
    Go,
    Java,
    Kotlin,
    Rust,
}

impl Keywords {
    fn of(language: Language) -> Option<Self> {
        match language {
            Language::Rust => Some(Self::Rust),
            Language::Kotlin => Some(Self::Kotlin),
            Language::CSharp => Some(Self::CSharp),
            Language::Go => Some(Self::Go),
            Language::Java => Some(Self::Java),
            Language::JavaScript
            | Language::TypeScript
            | Language::Markdown
            | Language::Json
            | Language::Sql => None,
        }
    }
}

/// Columns between tab stops when a tab is drawn. A terminal cell holds no
/// tab, so tabs become spaces before a line is coloured; left in, they draw
/// zero wide while the layout counts each as one column.
const TAB_WIDTH: usize = 4;

/// A line of a file in `language`: Markdown gets its own line-oriented pass,
/// everything else goes through [`highlight_code`].
pub(crate) fn highlight_line(
    code: &str,
    language: Option<Language>,
    default_style: Style,
) -> Vec<Span<'static>> {
    let code = expand_tabs(code);
    match language {
        Some(Language::Markdown) => highlight_markdown(&code, default_style),
        Some(Language::Json) => highlight_json(&code, default_style),
        Some(Language::Sql) => highlight_sql(&code, default_style),
        _ => highlight_code(&code, language, default_style),
    }
}

/// `code` with each tab replaced by the spaces up to the next tab stop.
fn expand_tabs(code: &str) -> Cow<'_, str> {
    if !code.contains('\t') {
        return Cow::Borrowed(code);
    }
    let mut out = String::with_capacity(code.len() + TAB_WIDTH);
    let mut column = 0usize;
    for ch in code.chars() {
        if ch == '\t' {
            let pad = TAB_WIDTH - column % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', pad));
            column += pad;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    Cow::Owned(out)
}

/// A line of JSON: keys apart from string values, and numbers and the three
/// words JSON has. A line may be any part of a document, so a string is a key
/// when a colon follows it on the line, whatever came before.
fn highlight_json(code: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut plain_start = 0usize;
    let mut chars = code.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        let end = if ch == '"' {
            string_end(code, idx + 1)
        } else if ch == '-' || ch.is_ascii_digit() || ch.is_ascii_alphabetic() {
            idx + code[idx..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
                .unwrap_or(code.len() - idx)
        } else {
            continue;
        };
        let token = &code[idx..end];
        let color = if ch == '"' {
            if code[end..].trim_start().starts_with(':') {
                Color::LightCyan
            } else {
                Color::LightYellow
            }
        } else if matches!(token, "true" | "false" | "null") {
            Color::Yellow
        } else if ch == '-' || ch.is_ascii_digit() {
            Color::LightYellow
        } else {
            while chars.peek().is_some_and(|(next, _)| *next < end) {
                chars.next();
            }
            continue;
        };
        push_plain_code(&mut spans, code, plain_start, idx, base);
        spans.push(Span::styled(token.to_string(), color_style(color, base)));
        while chars.peek().is_some_and(|(next, _)| *next < end) {
            chars.next();
        }
        plain_start = end;
    }
    push_plain_code(&mut spans, code, plain_start, code.len(), base);
    spans
}

/// A line of SQL. Keywords are matched in any case, as SQL is written both
/// ways. A line is lexed on its own, so a `/* */` comment that opens on it
/// runs to its close or the end of the line, and the lines inside one that
/// spans several are not known to be comment.
fn highlight_sql(code: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut plain_start = 0usize;
    let mut chars = code.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        let next = chars.peek().map(|(_, next)| *next);
        let (end, style) = if ch == '-' && next == Some('-') {
            (code.len(), Some(color_style(Color::DarkGray, base)))
        } else if ch == '/' && next == Some('*') {
            let end = code[idx + 2..]
                .find("*/")
                .map_or(code.len(), |close| idx + 2 + close + 2);
            (end, Some(color_style(Color::DarkGray, base)))
        } else if ch == '\'' {
            (
                sql_string_end(code, idx + 1),
                Some(color_style(Color::LightYellow, base)),
            )
        } else if ch == '"' || ch == '`' {
            // A quoted identifier is a name, whatever word it spells.
            let end = code[idx + 1..]
                .find(ch)
                .map_or(code.len(), |close| idx + close + 2);
            (end, None)
        } else if ch.is_ascii_digit() {
            let end = idx
                + code[idx..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
                    .unwrap_or(code.len() - idx);
            (end, Some(color_style(Color::LightYellow, base)))
        } else if is_ident_start(ch) {
            let end = idx
                + code[idx..]
                    .find(|c: char| !is_ident_continue(c))
                    .unwrap_or(code.len() - idx);
            let word = code[idx..end].to_ascii_lowercase();
            let style = if is_sql_keyword(&word) {
                Some(color_style(Color::Yellow, base).add_modifier(Modifier::BOLD))
            } else if is_sql_type(&word) {
                Some(color_style(Color::LightCyan, base))
            } else {
                function_style(code, end, base)
            };
            (end, style)
        } else {
            continue;
        };
        if let Some(style) = style {
            push_plain_code(&mut spans, code, plain_start, idx, base);
            spans.push(Span::styled(code[idx..end].to_string(), style));
            plain_start = end;
        }
        while chars.peek().is_some_and(|(next, _)| *next < end) {
            chars.next();
        }
    }
    push_plain_code(&mut spans, code, plain_start, code.len(), base);
    spans
}

/// The end of a SQL string whose body starts at `start`. SQL escapes a quote
/// by doubling it, so `''` inside the string does not close it.
fn sql_string_end(code: &str, start: usize) -> usize {
    let mut rest = start;
    while let Some(quote) = code[rest..].find('\'') {
        let after = rest + quote + 1;
        if code[after..].starts_with('\'') {
            rest = after + 1;
        } else {
            return after;
        }
    }
    code.len()
}

/// A line of code coloured by the code lexer: comments, strings, keywords,
/// types and calls. A language the lexer has no keywords for is one plain span.
pub(crate) fn highlight_code(
    code: &str,
    language: Option<Language>,
    default_style: Style,
) -> Vec<Span<'static>> {
    let code = &*expand_tabs(code);
    let Some(syntax) = language.and_then(Keywords::of) else {
        return vec![Span::styled(code.to_string(), default_style)];
    };

    let mut spans = Vec::new();
    let mut chars = code.char_indices().peekable();
    let mut plain_start = 0usize;
    while let Some((idx, ch)) = chars.next() {
        if ch == '/' && chars.peek().is_some_and(|(_, next)| *next == '/') {
            push_plain_code(&mut spans, code, plain_start, idx, default_style);
            spans.push(Span::styled(
                code[idx..].to_string(),
                color_style(Color::DarkGray, default_style),
            ));
            return spans;
        }
        let raw_string = ch == '`' && matches!(syntax, Keywords::Go);
        if ch == '"' || raw_string {
            push_plain_code(&mut spans, code, plain_start, idx, default_style);
            let end = if raw_string {
                // A Go raw string has no escapes and ends at the next backtick.
                code[idx + 1..]
                    .find('`')
                    .map_or(code.len(), |close| idx + close + 2)
            } else {
                string_end(code, idx + ch.len_utf8())
            };
            spans.push(Span::styled(
                code[idx..end].to_string(),
                color_style(Color::LightYellow, default_style),
            ));
            while chars.peek().is_some_and(|(next_idx, _)| *next_idx < end) {
                chars.next();
            }
            plain_start = end;
            continue;
        }
        if is_ident_start(ch) {
            let mut end = idx + ch.len_utf8();
            while let Some((next_idx, next)) = chars.peek().copied() {
                if !is_ident_continue(next) {
                    break;
                }
                chars.next();
                end = next_idx + next.len_utf8();
            }
            let ident = &code[idx..end];
            let style = keyword_style(ident, syntax, default_style)
                .or_else(|| type_style(ident, default_style))
                .or_else(|| function_style(code, end, default_style));
            if let Some(style) = style {
                push_plain_code(&mut spans, code, plain_start, idx, default_style);
                spans.push(Span::styled(ident.to_string(), style));
                plain_start = end;
            }
        }
    }
    push_plain_code(&mut spans, code, plain_start, code.len(), default_style);
    spans
}

/// Markdown is prose, not code, so it gets its own line-oriented pass: block
/// markers colour the whole line, everything else falls through to the inline
/// pass. Every character of `code` is preserved so diff widths stay intact.
fn highlight_markdown(code: &str, base: Style) -> Vec<Span<'static>> {
    let indent = code.len() - code.trim_start().len();
    let body = &code[indent..];
    let mut spans = Vec::new();
    if indent > 0 {
        spans.push(Span::styled(code[..indent].to_string(), base));
    }
    if body.is_empty() {
        return spans;
    }

    if is_markdown_fence(body) {
        spans.push(Span::styled(
            body.to_string(),
            color_style(Color::DarkGray, base),
        ));
        return spans;
    }
    if let Some((marker, text)) = markdown_heading(body) {
        spans.push(Span::styled(
            marker.to_string(),
            color_style(Color::LightBlue, base).add_modifier(Modifier::BOLD),
        ));
        spans.extend(markdown_inline(
            text,
            color_style(Color::LightCyan, base).add_modifier(Modifier::BOLD),
        ));
        return spans;
    }
    if body.starts_with('>') {
        spans.push(Span::styled(
            body.to_string(),
            color_style(Color::DarkGray, base),
        ));
        return spans;
    }
    if let Some((marker, text)) = markdown_bullet(body) {
        spans.push(Span::styled(
            marker.to_string(),
            color_style(Color::Yellow, base).add_modifier(Modifier::BOLD),
        ));
        spans.extend(markdown_inline(text, base));
        return spans;
    }

    spans.extend(markdown_inline(body, base));
    spans
}

fn is_markdown_fence(body: &str) -> bool {
    body.starts_with("```") || body.starts_with("~~~")
}

/// Splits `## Title` into its `## ` marker and the title text.
fn markdown_heading(body: &str) -> Option<(&str, &str)> {
    let hashes = body.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &body[hashes..];
    let spaces = rest.len() - rest.trim_start_matches(' ').len();
    (spaces > 0 || rest.is_empty()).then(|| body.split_at(hashes + spaces))
}

/// Splits `- item` or `3. item` into its marker (including trailing space) and
/// the item text.
fn markdown_bullet(body: &str) -> Option<(&str, &str)> {
    let marker_len = if matches!(body.as_bytes().first(), Some(b'-' | b'*' | b'+')) {
        1
    } else {
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !matches!(body.as_bytes().get(digits), Some(b'.' | b')')) {
            return None;
        }
        digits + 1
    };
    let rest = &body[marker_len..];
    let spaces = rest.len() - rest.trim_start_matches(' ').len();
    (spaces > 0).then(|| body.split_at(marker_len + spaces))
}

fn markdown_inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    let mut plain = String::new();
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('`')
            && let Some(end) = inner.find('`')
        {
            push_markdown_plain(&mut spans, &mut plain, base);
            spans.push(Span::styled(
                rest[..end + 2].to_string(),
                color_style(Color::LightYellow, base),
            ));
            rest = &inner[end + 1..];
            continue;
        }
        if let Some(inner) = rest.strip_prefix("**")
            && let Some(end) = inner.find("**")
        {
            push_markdown_plain(&mut spans, &mut plain, base);
            spans.push(Span::styled(
                rest[..end + 4].to_string(),
                base.add_modifier(Modifier::BOLD),
            ));
            rest = &inner[end + 2..];
            continue;
        }
        if let Some(end) = markdown_link_end(rest) {
            push_markdown_plain(&mut spans, &mut plain, base);
            spans.push(Span::styled(
                rest[..end].to_string(),
                color_style(Color::LightBlue, base),
            ));
            rest = &rest[end..];
            continue;
        }
        let ch = rest.chars().next().unwrap_or_default();
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    push_markdown_plain(&mut spans, &mut plain, base);
    spans
}

/// Length of the `[text](target)` link starting at `rest`, if there is one.
fn markdown_link_end(rest: &str) -> Option<usize> {
    let inner = rest.strip_prefix('[')?;
    let label_end = inner.find(']')?;
    let target = inner[label_end + 1..].strip_prefix('(')?;
    let target_end = target.find(')')?;
    Some(label_end + target_end + 4)
}

fn push_markdown_plain(spans: &mut Vec<Span<'static>>, plain: &mut String, base: Style) {
    if !plain.is_empty() {
        spans.push(Span::styled(std::mem::take(plain), base));
    }
}

fn push_plain_code(
    spans: &mut Vec<Span<'static>>,
    code: &str,
    start: usize,
    end: usize,
    style: Style,
) {
    if start < end {
        spans.push(Span::styled(code[start..end].to_string(), style));
    }
}

fn string_end(code: &str, start: usize) -> usize {
    let mut escaped = false;
    for (idx, ch) in code[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return start + idx + ch.len_utf8();
        }
    }
    code.len()
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn color_style(color: Color, base: Style) -> Style {
    let style = Style::default().fg(color);
    if let Some(bg) = base.bg {
        style.bg(bg)
    } else {
        style
    }
}

fn type_style(word: &str, base: Style) -> Option<Style> {
    word.chars()
        .next()
        .is_some_and(char::is_uppercase)
        .then_some(color_style(Color::LightCyan, base))
}

fn function_style(code: &str, ident_end: usize, base: Style) -> Option<Style> {
    let next = code[ident_end..].chars().find(|ch| !ch.is_whitespace())?;
    (next == '(').then_some(color_style(Color::LightMagenta, base))
}

fn keyword_style(word: &str, syntax: Keywords, base: Style) -> Option<Style> {
    let keyword = match syntax {
        Keywords::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "use"
                | "where"
                | "while"
        ),
        Keywords::CSharp => matches!(
            word,
            "abstract"
                | "as"
                | "async"
                | "await"
                | "base"
                | "bool"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delegate"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "event"
                | "false"
                | "finally"
                | "for"
                | "foreach"
                | "get"
                | "if"
                | "in"
                | "int"
                | "interface"
                | "internal"
                | "is"
                | "lock"
                | "long"
                | "namespace"
                | "new"
                | "null"
                | "object"
                | "out"
                | "override"
                | "params"
                | "partial"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "record"
                | "ref"
                | "return"
                | "sealed"
                | "set"
                | "static"
                | "string"
                | "struct"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typeof"
                | "using"
                | "var"
                | "virtual"
                | "void"
                | "when"
                | "where"
                | "while"
                | "yield"
        ),
        Keywords::Java => matches!(
            word,
            "abstract"
                | "assert"
                | "boolean"
                | "break"
                | "byte"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extends"
                | "false"
                | "final"
                | "finally"
                | "float"
                | "for"
                | "if"
                | "implements"
                | "import"
                | "instanceof"
                | "int"
                | "interface"
                | "long"
                | "module"
                | "native"
                | "new"
                | "null"
                | "package"
                | "permits"
                | "private"
                | "protected"
                | "public"
                | "record"
                | "return"
                | "sealed"
                | "short"
                | "static"
                | "super"
                | "switch"
                | "synchronized"
                | "this"
                | "throw"
                | "throws"
                | "transient"
                | "true"
                | "try"
                | "var"
                | "void"
                | "volatile"
                | "when"
                | "while"
                | "yield"
        ),
        Keywords::Go => matches!(
            word,
            "any"
                | "bool"
                | "break"
                | "byte"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "error"
                | "fallthrough"
                | "false"
                | "float64"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "int"
                | "int64"
                | "interface"
                | "iota"
                | "map"
                | "nil"
                | "package"
                | "range"
                | "return"
                | "rune"
                | "select"
                | "string"
                | "struct"
                | "switch"
                | "true"
                | "type"
                | "var"
        ),
        Keywords::Kotlin => matches!(
            word,
            "as" | "class"
                | "data"
                | "else"
                | "false"
                | "fun"
                | "if"
                | "in"
                | "interface"
                | "is"
                | "null"
                | "object"
                | "override"
                | "private"
                | "return"
                | "suspend"
                | "true"
                | "val"
                | "var"
                | "when"
                | "while"
        ),
    };
    keyword.then_some(color_style(Color::Yellow, base).add_modifier(Modifier::BOLD))
}

/// A SQL keyword, given in lower case.
fn is_sql_keyword(word: &str) -> bool {
    matches!(
        word,
        "add"
            | "after"
            | "all"
            | "alter"
            | "and"
            | "as"
            | "asc"
            | "before"
            | "begin"
            | "between"
            | "by"
            | "cascade"
            | "case"
            | "check"
            | "column"
            | "commit"
            | "conflict"
            | "constraint"
            | "create"
            | "cross"
            | "default"
            | "delete"
            | "desc"
            | "distinct"
            | "do"
            | "drop"
            | "else"
            | "end"
            | "except"
            | "exists"
            | "false"
            | "foreign"
            | "from"
            | "full"
            | "function"
            | "grant"
            | "group"
            | "having"
            | "if"
            | "ilike"
            | "in"
            | "index"
            | "inner"
            | "insert"
            | "intersect"
            | "into"
            | "is"
            | "join"
            | "key"
            | "lateral"
            | "left"
            | "like"
            | "limit"
            | "not"
            | "nothing"
            | "null"
            | "offset"
            | "on"
            | "or"
            | "order"
            | "outer"
            | "over"
            | "partition"
            | "primary"
            | "procedure"
            | "recursive"
            | "references"
            | "rename"
            | "replace"
            | "returning"
            | "returns"
            | "revoke"
            | "right"
            | "rollback"
            | "schema"
            | "select"
            | "sequence"
            | "set"
            | "table"
            | "then"
            | "to"
            | "transaction"
            | "trigger"
            | "true"
            | "truncate"
            | "union"
            | "unique"
            | "update"
            | "using"
            | "values"
            | "view"
            | "when"
            | "where"
            | "with"
    )
}

/// A SQL column type, given in lower case.
fn is_sql_type(word: &str) -> bool {
    matches!(
        word,
        "bigint"
            | "bigserial"
            | "blob"
            | "bool"
            | "boolean"
            | "bytea"
            | "char"
            | "date"
            | "decimal"
            | "double"
            | "float"
            | "int"
            | "integer"
            | "interval"
            | "json"
            | "jsonb"
            | "numeric"
            | "real"
            | "serial"
            | "smallint"
            | "text"
            | "time"
            | "timestamp"
            | "timestamptz"
            | "uuid"
            | "varchar"
    )
}
