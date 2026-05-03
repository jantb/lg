use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

const CODE_BG: Color = Color::Rgb(28, 31, 38);

#[derive(Clone, Copy)]
enum Syntax {
    Kotlin,
    Rust,
}

pub fn render(text: &str, prefix: &str, wrap_width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut code: Option<Option<Syntax>> = None;

    for raw in text.lines() {
        if let Some(lang) = code_fence_language(raw) {
            if code.is_some() {
                lines.push(code_rule(prefix, "└─"));
                code = None;
            } else {
                let syntax = language_syntax(lang);
                let label = if lang.is_empty() {
                    "code".to_string()
                } else {
                    format!("code {lang}")
                };
                lines.push(code_rule(prefix, &format!("┌─ {label}")));
                code = Some(syntax);
            }
            continue;
        }

        if let Some(syntax) = code {
            let mut spans = prefixed(prefix);
            spans.push(Span::styled("  ", Style::default().bg(CODE_BG)));
            spans.extend(code_spans(
                raw,
                syntax,
                Style::default().fg(Color::Gray).bg(CODE_BG),
            ));
            lines.push(Line::from(spans));
            continue;
        }

        lines.extend(markdown_lines(raw, prefix, wrap_width));
    }

    if code.is_some() {
        lines.push(code_rule(prefix, "└─"));
    }

    lines
}

fn markdown_lines(raw: &str, prefix: &str, wrap_width: u16) -> Vec<Line<'static>> {
    let trimmed = raw.trim_start();
    let indent_len = raw.len().saturating_sub(trimmed.len()).min(8);
    let prefix_width = prefix.chars().count() + indent_len;

    if trimmed.is_empty() {
        let mut spans = prefixed(prefix);
        if indent_len > 0 {
            spans.push(Span::raw(" ".repeat(indent_len)));
        }
        return vec![Line::from(spans)];
    }

    if let Some((level, heading)) = heading(trimmed) {
        let mut spans = prefixed(prefix);
        if indent_len > 0 {
            spans.push(Span::raw(" ".repeat(indent_len)));
        }
        spans.push(Span::styled(
            format!("{} ", "▸".repeat(level.min(3))),
            Style::default().fg(Color::LightBlue),
        ));
        spans.extend(inline_spans(
            heading,
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ));
        return vec![Line::from(spans)];
    }

    if let Some(rest) = unordered_bullet(trimmed) {
        let marker = Span::styled("• ", Style::default().fg(Color::Yellow));
        return wrap_with_marker(
            prefix,
            indent_len,
            marker,
            2,
            rest,
            wrap_width,
            prefix_width,
        );
    }

    if let Some((number, rest)) = ordered_bullet(trimmed) {
        let marker_text = format!("{number}. ");
        let marker_width = marker_text.chars().count();
        let marker = Span::styled(
            marker_text,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        return wrap_with_marker(
            prefix,
            indent_len,
            marker,
            marker_width,
            rest,
            wrap_width,
            prefix_width,
        );
    }

    wrap_plain(prefix, indent_len, trimmed, wrap_width, prefix_width)
}

fn wrap_with_marker(
    prefix: &str,
    indent_len: usize,
    marker: Span<'static>,
    marker_width: usize,
    content: &str,
    wrap_width: u16,
    prefix_width: usize,
) -> Vec<Line<'static>> {
    let available = (wrap_width as usize)
        .saturating_sub(prefix_width)
        .saturating_sub(marker_width)
        .max(1);
    let chunks = word_wrap(content, available);
    let style = Style::default().fg(Color::Gray);

    chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk)| {
            let mut spans = prefixed(prefix);
            if indent_len > 0 {
                spans.push(Span::raw(" ".repeat(indent_len)));
            }
            if idx == 0 {
                spans.push(marker.clone());
            } else {
                spans.push(Span::raw(" ".repeat(marker_width)));
            }
            spans.extend(inline_spans(&chunk, style));
            Line::from(spans)
        })
        .collect()
}

fn wrap_plain(
    prefix: &str,
    indent_len: usize,
    content: &str,
    wrap_width: u16,
    prefix_width: usize,
) -> Vec<Line<'static>> {
    let available = (wrap_width as usize).saturating_sub(prefix_width).max(1);
    let chunks = word_wrap(content, available);
    let style = Style::default().fg(Color::Gray);

    chunks
        .into_iter()
        .map(|chunk| {
            let mut spans = prefixed(prefix);
            if indent_len > 0 {
                spans.push(Span::raw(" ".repeat(indent_len)));
            }
            spans.extend(inline_spans(&chunk, style));
            Line::from(spans)
        })
        .collect()
}

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let w = word.chars().count();
        if current.is_empty() {
            current.push_str(word);
            current_width = w;
        } else if current_width + 1 + w <= width {
            current.push(' ');
            current.push_str(word);
            current_width += 1 + w;
        } else {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
            current_width = w;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn prefixed(prefix: &str) -> Vec<Span<'static>> {
    vec![Span::styled(
        prefix.to_string(),
        Style::default().fg(Color::DarkGray),
    )]
}

fn code_rule(prefix: &str, label: &str) -> Line<'static> {
    let mut spans = prefixed(prefix);
    spans.push(Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(Color::LightBlue)
            .bg(CODE_BG)
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

fn code_fence_language(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix("```").map(str::trim)
}

fn language_syntax(lang: &str) -> Option<Syntax> {
    match lang.to_ascii_lowercase().as_str() {
        "rs" | "rust" => Some(Syntax::Rust),
        "kt" | "kts" | "kotlin" => Some(Syntax::Kotlin),
        _ => None,
    }
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || !line[level..].starts_with(' ') {
        return None;
    }
    Some((level, line[level + 1..].trim()))
}

fn unordered_bullet(line: &str) -> Option<&str> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(str::trim)
}

fn ordered_bullet(line: &str) -> Option<(&str, &str)> {
    let (number, rest) = line.split_once(". ")?;
    number
        .chars()
        .all(|ch| ch.is_ascii_digit())
        .then_some((number, rest.trim()))
}

fn inline_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let bold = rest.find("**");
        let code = rest.find('`');
        match (bold, code) {
            (Some(bold), Some(code)) if code < bold => {
                push_inline_code(&mut spans, &mut rest, code, base);
            }
            (Some(bold), _) => {
                push_bold(&mut spans, &mut rest, bold, base);
            }
            (None, Some(code)) => {
                push_inline_code(&mut spans, &mut rest, code, base);
            }
            (None, None) => {
                spans.push(Span::styled(rest.to_string(), base));
                break;
            }
        }
    }
    spans
}

fn push_bold(spans: &mut Vec<Span<'static>>, rest: &mut &str, start: usize, base: Style) {
    if start > 0 {
        spans.push(Span::styled(rest[..start].to_string(), base));
    }
    let content_start = start + 2;
    let Some(end) = rest[content_start..].find("**") else {
        spans.push(Span::styled(rest[start..].to_string(), base));
        *rest = "";
        return;
    };
    let content_end = content_start + end;
    spans.push(Span::styled(
        rest[content_start..content_end].to_string(),
        base.fg(Color::White).add_modifier(Modifier::BOLD),
    ));
    *rest = &rest[content_end + 2..];
}

fn push_inline_code(spans: &mut Vec<Span<'static>>, rest: &mut &str, start: usize, base: Style) {
    if start > 0 {
        spans.push(Span::styled(rest[..start].to_string(), base));
    }
    let content_start = start + 1;
    let Some(end) = rest[content_start..].find('`') else {
        spans.push(Span::styled(rest[start..].to_string(), base));
        *rest = "";
        return;
    };
    let content_end = content_start + end;
    spans.push(Span::styled(
        rest[content_start..content_end].to_string(),
        Style::default()
            .fg(Color::LightYellow)
            .bg(Color::Rgb(42, 44, 52)),
    ));
    *rest = &rest[content_end + 1..];
}

fn code_spans(code: &str, syntax: Option<Syntax>, base: Style) -> Vec<Span<'static>> {
    let Some(syntax) = syntax else {
        return vec![Span::styled(code.to_string(), base)];
    };

    let mut spans = Vec::new();
    let mut chars = code.char_indices().peekable();
    let mut plain_start = 0usize;
    while let Some((idx, ch)) = chars.next() {
        if ch == '/' && chars.peek().is_some_and(|(_, next)| *next == '/') {
            push_plain_code(&mut spans, code, plain_start, idx, base);
            spans.push(Span::styled(
                code[idx..].to_string(),
                style_with_bg(Color::DarkGray, base),
            ));
            return spans;
        }
        if ch == '"' {
            push_plain_code(&mut spans, code, plain_start, idx, base);
            let end = string_end(code, idx + ch.len_utf8());
            spans.push(Span::styled(
                code[idx..end].to_string(),
                style_with_bg(Color::LightYellow, base),
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
            let style = keyword_style(ident, syntax, base)
                .or_else(|| type_style(ident, base))
                .or_else(|| function_style(code, end, base));
            if let Some(style) = style {
                push_plain_code(&mut spans, code, plain_start, idx, base);
                spans.push(Span::styled(ident.to_string(), style));
                plain_start = end;
            }
        }
    }
    push_plain_code(&mut spans, code, plain_start, code.len(), base);
    spans
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

fn style_with_bg(color: Color, base: Style) -> Style {
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
        .then_some(style_with_bg(Color::LightCyan, base))
}

fn function_style(code: &str, ident_end: usize, base: Style) -> Option<Style> {
    let next = code[ident_end..].chars().find(|ch| !ch.is_whitespace())?;
    (next == '(').then_some(style_with_bg(Color::LightMagenta, base))
}

fn keyword_style(word: &str, syntax: Syntax, base: Style) -> Option<Style> {
    let keyword = match syntax {
        Syntax::Rust => matches!(
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
        Syntax::Kotlin => matches!(
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
    keyword.then_some(style_with_bg(Color::Yellow, base).add_modifier(Modifier::BOLD))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn word_wrap_keeps_short_text_in_one_chunk() {
        assert_eq!(
            word_wrap("hello world", 40),
            vec!["hello world".to_string()]
        );
    }

    #[test]
    fn word_wrap_breaks_at_word_boundary() {
        assert_eq!(
            word_wrap("alpha beta gamma delta", 11),
            vec!["alpha beta".to_string(), "gamma delta".to_string()],
        );
    }

    #[test]
    fn word_wrap_keeps_oversize_word_on_its_own_line() {
        // A word longer than `width` should not be split mid-character;
        // it lives alone on its line so styling (bold/code) stays intact.
        assert_eq!(
            word_wrap("ab supercalifragilistic done", 6),
            vec![
                "ab".to_string(),
                "supercalifragilistic".to_string(),
                "done".to_string(),
            ],
        );
    }

    #[test]
    fn word_wrap_handles_empty_input() {
        assert_eq!(word_wrap("", 40), vec![String::new()]);
    }

    #[test]
    fn bullet_wrap_hangs_continuation_under_bullet_text() {
        // Width is small enough to force a wrap; the bullet marker must only
        // appear on the first line, and the wrapped continuation must align
        // with the bullet text (i.e. start with the prefix + two spaces, not
        // the bullet itself).
        let lines = render(
            "- alpha beta gamma delta epsilon",
            "│ ",
            18, // 18 - 2 (prefix) - 2 (marker) = 14 cols of body
        );
        assert!(lines.len() >= 2, "expected wrapped output, got {lines:?}");
        assert_eq!(line_text(&lines[0]), "│ • alpha beta");
        // All continuation lines align with the bullet text: prefix + two
        // spaces (the width of "• "), no bullet glyph repeated.
        for line in &lines[1..] {
            let text = line_text(line);
            assert!(
                text.starts_with("│   "),
                "expected hanging indent under '• ', got {text:?}",
            );
            assert!(
                !text.contains('•'),
                "continuation line should not repeat the bullet: {text:?}",
            );
        }
    }

    #[test]
    fn ordered_bullet_wrap_uses_marker_width_for_hanging_indent() {
        let lines = render("10. one two three four five", "", 12);
        assert!(lines.len() >= 2);
        assert!(line_text(&lines[0]).starts_with("10. "));
        // Continuation aligns with first character after "10. " (4 spaces).
        for line in &lines[1..] {
            assert!(
                line_text(line).starts_with("    "),
                "expected hanging indent under '10. ', got {:?}",
                line_text(line)
            );
        }
    }

    #[test]
    fn plain_paragraph_wraps_at_word_boundary() {
        let lines = render("just a paragraph that needs wrapping", "│ ", 16);
        assert!(lines.len() > 1);
        assert_eq!(line_text(&lines[0]), "│ just a");
        assert_eq!(line_text(&lines[1]), "│ paragraph that");
    }

    #[test]
    fn short_plain_paragraph_is_emitted_as_single_line() {
        let lines = render("just a paragraph", "│ ", 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "│ just a paragraph");
    }

    #[test]
    fn heading_includes_marker_and_text() {
        let lines = render("## Section title", "", 80);
        assert_eq!(lines.len(), 1);
        assert!(line_text(&lines[0]).contains("Section title"));
    }
}
