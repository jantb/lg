use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

mod side_by_side;
mod syntax;
use side_by_side::*;
use syntax::*;
pub use syntax::{Token, diff_header_path, diff_line_tokens};

/// Colorize a single diff line into a styled `Line`.
pub fn highlight_diff_line(line: &str) -> Line<'_> {
    highlight_diff_line_for_syntax(line, None)
}

pub fn highlight_diff_line_for_path<'a>(line: &'a str, path: &str) -> Line<'a> {
    highlight_diff_line_for_syntax(line, path_syntax(path))
}

pub fn highlight_source_line_for_path<'a>(line: &'a str, path: &str) -> Line<'a> {
    Line::from(highlight_code(
        line,
        path_syntax(path),
        Style::default().fg(Color::Gray),
    ))
}

pub fn highlight_diff_text(text: &str) -> Vec<Line<'_>> {
    unified_diff_lines(text)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}

/// A unified diff wrapped to `width` rather than run off the right edge of the
/// pane. Continuation rows are indented past the line-number gutter so the code
/// column stays aligned.
pub fn highlight_diff_text_wrapped(text: &str, width: u16) -> Vec<Line<'static>> {
    unified_diff_lines(text)
        .into_iter()
        .flat_map(|(line, gutter)| wrap_line(line, width as usize, gutter))
        .collect()
}

/// Rows a unified diff takes once wrapped. The scroll bound has to count what
/// [`highlight_diff_text_wrapped`] actually draws, or the tail of a diff with
/// long lines cannot be scrolled to.
pub fn diff_text_line_count(text: &str, width: u16) -> usize {
    unified_diff_lines(text)
        .iter()
        .map(|(line, gutter)| wrapped_row_count(spans_width(&line.spans), *gutter, width as usize))
        .sum()
}

/// One diff line, wrapped to `width`. For callers that render diff bodies a
/// line at a time rather than as one block of text.
pub fn highlight_diff_line_wrapped_for_path(
    line: &str,
    path: &str,
    width: u16,
) -> Vec<Line<'static>> {
    wrap_line(highlight_diff_line_for_path(line, path), width as usize, 0)
}

/// Each rendered line paired with the gutter width its continuation rows have
/// to clear.
fn unified_diff_lines(text: &str) -> Vec<(Line<'_>, usize)> {
    let mut syntax = None;
    let mut line_numbers = None;
    text.split('\n')
        .map(|line| {
            if let Some(next) = diff_line_syntax(line) {
                syntax = Some(next);
            }
            if let Some((old_line, new_line)) = parse_hunk_line_numbers(line) {
                line_numbers = Some(DiffLineNumbers { old_line, new_line });
                return (highlight_diff_line_for_syntax(line, syntax), 0);
            }
            let highlighted = highlight_diff_line_for_syntax(line, syntax);
            if let Some(numbers) = line_numbers.as_mut()
                && let Some(kind) = diff_content_kind(line)
            {
                return (
                    add_diff_line_numbers(highlighted, numbers, kind),
                    DIFF_NUMBER_GUTTER,
                );
            }
            (highlighted, 0)
        })
        .collect()
}

pub fn highlight_side_by_side_diff_text(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut renderer = SideBySideDiffRenderer::new(width as usize);
    for line in text.lines() {
        renderer.push_line(line);
    }
    renderer.finish()
}

pub fn highlight_side_by_side_diff_text_for_path(
    text: &str,
    width: u16,
    path: &str,
) -> Vec<Line<'static>> {
    let mut renderer = SideBySideDiffRenderer::new(width as usize);
    renderer.syntax = path_syntax(path);
    for line in text.lines() {
        renderer.push_line(line);
    }
    renderer.finish()
}

pub fn side_by_side_diff_line_count(text: &str, width: u16) -> usize {
    highlight_side_by_side_diff_text(text, width).len()
}

#[derive(Clone, Copy)]
enum Syntax {
    CSharp,
    Kotlin,
    Markdown,
    Rust,
}

fn highlight_diff_line_for_syntax(line: &str, syntax: Option<Syntax>) -> Line<'_> {
    if matches!(line, "Message:" | "Files changed:" | "Patch:") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if line.starts_with("commit ") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if line.starts_with("Author:") || line.starts_with("Date:") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Gray)));
    }
    if line.starts_with("+++") || line.starts_with("---") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = line.strip_prefix('+') {
        let base_style = Style::default().fg(Color::Gray).bg(DIFF_ADDED_BG);
        let mut spans = vec![Span::styled(
            "+",
            Style::default()
                .fg(Color::Green)
                .bg(DIFF_ADDED_BG)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(highlight_code(rest, syntax, base_style));
        return Line::from(spans);
    }
    if let Some(rest) = line.strip_prefix('-') {
        let base_style = Style::default().fg(Color::Gray).bg(DIFF_REMOVED_BG);
        let mut spans = vec![Span::styled(
            "-",
            Style::default()
                .fg(Color::Red)
                .bg(DIFF_REMOVED_BG)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(highlight_code(rest, syntax, base_style));
        return Line::from(spans);
    }
    if line.starts_with("@@") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Cyan)));
    }
    if line.starts_with("diff --git ") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(highlight_code(line, syntax, Style::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled_text(line: &Line<'_>, color: Color) -> String {
        line.spans
            .iter()
            .filter(|span| span.style.fg == Some(color))
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_diff_line_tells_its_characters_apart_by_what_they_are() {
        let line = "+    let name = \"ok\"; // note";
        let tokens = diff_line_tokens(line, "src/main.rs");
        assert_eq!(tokens.len(), line.chars().count(), "one per character");
        let of = |needle: &str, token: Token| {
            let at = line.find(needle).expect("the line has it");
            tokens[at..at + needle.chars().count()]
                .iter()
                .all(|t| *t == token)
        };
        assert!(of("+", Token::Marker), "{tokens:?}");
        assert!(of("let", Token::Keyword), "{tokens:?}");
        assert!(of("\"ok\"", Token::Literal), "{tokens:?}");
        assert!(of("// note", Token::Comment), "{tokens:?}");
        assert!(of("name", Token::Plain), "{tokens:?}");

        // A file with no syntax of its own still reads as a diff.
        let plain = diff_line_tokens("-let name = 1;", "notes.txt");
        assert_eq!(plain[0], Token::Marker);
        assert!(plain[1..].iter().all(|t| *t == Token::Plain), "{plain:?}");
    }

    #[test]
    fn csharp_source_line_highlights_keywords_types_and_strings() {
        let line = highlight_source_line_for_path(
            "public async Task<Order> Load(string id) { return \"ok\"; }",
            "src/Orders/OrderService.cs",
        );

        let keywords = styled_text(&line, Color::Yellow);
        assert!(keywords.contains("public"), "{keywords}");
        assert!(keywords.contains("async"), "{keywords}");
        assert!(keywords.contains("return"), "{keywords}");
        assert!(styled_text(&line, Color::LightCyan).contains("Task"));
        assert!(styled_text(&line, Color::LightYellow).contains("\"ok\""));
    }

    #[test]
    fn markdown_source_line_highlights_headings_code_and_links() {
        let heading = highlight_source_line_for_path("## Setup `lg`", "docs/guide.md");
        assert!(
            heading
                .spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(styled_text(&heading, Color::LightYellow).contains("`lg`"));

        let bullet = highlight_source_line_for_path(
            "- see [the docs](https://example.com)",
            "docs/guide.md",
        );
        assert!(styled_text(&bullet, Color::Yellow).contains('-'));
        assert!(styled_text(&bullet, Color::LightBlue).contains("[the docs](https://example.com)"));
    }

    #[test]
    fn markdown_highlighting_preserves_every_character() {
        let source = "1. **bold** text with `code` and a [link](url) ~ done";
        let line = highlight_source_line_for_path(source, "README.md");

        let rendered: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(rendered, source);
    }

    #[test]
    fn csharp_comment_is_dimmed() {
        let line = highlight_source_line_for_path("var x = 1; // note", "Program.csx");

        assert!(styled_text(&line, Color::DarkGray).contains("// note"));
    }
}
