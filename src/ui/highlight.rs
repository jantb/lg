//! Syntax colouring for a line of code, shared by the diff, the source view
//! and the code blocks of rendered Markdown.

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
    Kotlin,
    Rust,
}

impl Keywords {
    fn of(language: Language) -> Option<Self> {
        match language {
            Language::Rust => Some(Self::Rust),
            Language::Kotlin => Some(Self::Kotlin),
            Language::CSharp => Some(Self::CSharp),
            Language::Java | Language::JavaScript | Language::TypeScript | Language::Markdown => {
                None
            }
        }
    }
}

/// A line of a file in `language`: Markdown gets its own line-oriented pass,
/// everything else goes through [`highlight_code`].
pub(crate) fn highlight_line(
    code: &str,
    language: Option<Language>,
    default_style: Style,
) -> Vec<Span<'static>> {
    if language == Some(Language::Markdown) {
        return highlight_markdown(code, default_style);
    }
    highlight_code(code, language, default_style)
}

/// A line of code coloured by the code lexer: comments, strings, keywords,
/// types and calls. A language the lexer has no keywords for is one plain span.
pub(crate) fn highlight_code(
    code: &str,
    language: Option<Language>,
    default_style: Style,
) -> Vec<Span<'static>> {
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
        if ch == '"' {
            push_plain_code(&mut spans, code, plain_start, idx, default_style);
            let end = string_end(code, idx + ch.len_utf8());
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
