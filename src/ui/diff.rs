use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

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
    let mut syntax = None;
    let mut line_numbers = None;
    text.split('\n')
        .map(|line| {
            if let Some(next) = diff_line_syntax(line) {
                syntax = Some(next);
            }
            if let Some((old_line, new_line)) = parse_hunk_line_numbers(line) {
                line_numbers = Some(DiffLineNumbers { old_line, new_line });
                return highlight_diff_line_for_syntax(line, syntax);
            }
            let highlighted = highlight_diff_line_for_syntax(line, syntax);
            if let Some(numbers) = line_numbers.as_mut()
                && let Some(kind) = diff_content_kind(line)
            {
                return add_diff_line_numbers(highlighted, numbers, kind);
            }
            highlighted
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

const DIFF_ADDED_BG: Color = Color::Rgb(24, 54, 34);
const DIFF_REMOVED_BG: Color = Color::Rgb(60, 28, 38);
const SIDE_SEPARATOR: &str = " | ";
const SIDE_NUMBER_WIDTH: usize = 4;

#[derive(Clone, Copy)]
enum Syntax {
    Kotlin,
    Rust,
}

struct DiffLineNumbers {
    old_line: u32,
    new_line: u32,
}

#[derive(Clone, Copy)]
enum DiffContentKind {
    Context,
    Added,
    Removed,
}

struct SideDiffCell {
    number: u32,
    text: String,
    kind: DiffContentKind,
}

struct SideBySideDiffRenderer {
    width: usize,
    syntax: Option<Syntax>,
    numbers: Option<DiffLineNumbers>,
    in_hunk: bool,
    lines: Vec<Line<'static>>,
    pending_removed: Vec<SideDiffCell>,
    pending_added: Vec<SideDiffCell>,
}

impl SideBySideDiffRenderer {
    fn new(width: usize) -> Self {
        Self {
            width,
            syntax: None,
            numbers: None,
            in_hunk: false,
            lines: Vec::new(),
            pending_removed: Vec::new(),
            pending_added: Vec::new(),
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_change_run();
        self.lines
    }

    fn push_line(&mut self, line: &str) {
        if let Some(next) = diff_line_syntax(line) {
            self.syntax = Some(next);
        }

        if let Some((old_line, new_line)) = parse_hunk_line_numbers(line) {
            self.flush_change_run();
            self.numbers = Some(DiffLineNumbers { old_line, new_line });
            self.in_hunk = true;
            self.push_full_line(line);
            return;
        }

        if line.starts_with("diff --git ") || line.starts_with("---") || line.starts_with("+++") {
            self.in_hunk = false;
            self.push_full_line(line);
            return;
        }

        if self.in_hunk
            && let Some(kind) = diff_content_kind(line)
            && self.numbers.is_some()
        {
            match kind {
                DiffContentKind::Context => {
                    self.flush_change_run();
                    let numbers = self.numbers.as_mut().expect("hunk line numbers");
                    let old = numbers.old_line;
                    let new = numbers.new_line;
                    numbers.old_line = numbers.old_line.saturating_add(1);
                    numbers.new_line = numbers.new_line.saturating_add(1);
                    let text = line.strip_prefix(' ').unwrap_or(line).to_string();
                    self.lines.push(render_side_by_side_row(
                        Some(&SideDiffCell {
                            number: old,
                            text: text.clone(),
                            kind,
                        }),
                        Some(&SideDiffCell {
                            number: new,
                            text,
                            kind,
                        }),
                        self.width,
                        self.syntax,
                    ));
                }
                DiffContentKind::Added => {
                    let numbers = self.numbers.as_mut().expect("hunk line numbers");
                    let number = numbers.new_line;
                    numbers.new_line = numbers.new_line.saturating_add(1);
                    self.pending_added.push(SideDiffCell {
                        number,
                        text: line.strip_prefix('+').unwrap_or(line).to_string(),
                        kind,
                    });
                }
                DiffContentKind::Removed => {
                    if !self.pending_added.is_empty() {
                        self.flush_change_run();
                    }
                    let numbers = self.numbers.as_mut().expect("hunk line numbers");
                    let number = numbers.old_line;
                    numbers.old_line = numbers.old_line.saturating_add(1);
                    self.pending_removed.push(SideDiffCell {
                        number,
                        text: line.strip_prefix('-').unwrap_or(line).to_string(),
                        kind,
                    });
                }
            }
            return;
        }

        self.push_full_line(line);
    }

    fn push_full_line(&mut self, line: &str) {
        self.flush_change_run();
        self.lines
            .push(render_full_side_by_side_line(line, self.width, self.syntax));
    }

    fn flush_change_run(&mut self) {
        let rows = self.pending_removed.len().max(self.pending_added.len());
        for idx in 0..rows {
            self.lines.push(render_side_by_side_row(
                self.pending_removed.get(idx),
                self.pending_added.get(idx),
                self.width,
                self.syntax,
            ));
        }
        self.pending_removed.clear();
        self.pending_added.clear();
    }
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

fn parse_hunk_line_numbers(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    Some((parse_hunk_start(old)?, parse_hunk_start(new)?))
}

fn parse_hunk_start(part: &str) -> Option<u32> {
    part.split(',').next()?.parse().ok()
}

fn diff_content_kind(line: &str) -> Option<DiffContentKind> {
    if line.starts_with("+++") || line.starts_with("---") {
        return None;
    }
    if line.starts_with('+') {
        Some(DiffContentKind::Added)
    } else if line.starts_with('-') {
        Some(DiffContentKind::Removed)
    } else if line.starts_with(' ') || line.is_empty() {
        Some(DiffContentKind::Context)
    } else {
        None
    }
}

fn add_diff_line_numbers<'a>(
    line: Line<'a>,
    numbers: &mut DiffLineNumbers,
    kind: DiffContentKind,
) -> Line<'a> {
    let (old, new, old_style, new_style) = match kind {
        DiffContentKind::Context => {
            let old = numbers.old_line;
            let new = numbers.new_line;
            numbers.old_line = numbers.old_line.saturating_add(1);
            numbers.new_line = numbers.new_line.saturating_add(1);
            (
                Some(old),
                Some(new),
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::DarkGray),
            )
        }
        DiffContentKind::Added => {
            let new = numbers.new_line;
            numbers.new_line = numbers.new_line.saturating_add(1);
            (
                None,
                Some(new),
                Style::default().fg(Color::DarkGray).bg(DIFF_ADDED_BG),
                Style::default().fg(Color::LightGreen).bg(DIFF_ADDED_BG),
            )
        }
        DiffContentKind::Removed => {
            let old = numbers.old_line;
            numbers.old_line = numbers.old_line.saturating_add(1);
            (
                Some(old),
                None,
                Style::default().fg(Color::LightRed).bg(DIFF_REMOVED_BG),
                Style::default().fg(Color::DarkGray).bg(DIFF_REMOVED_BG),
            )
        }
    };
    let mut spans = vec![
        Span::styled(
            format!("{:>4}", old.map_or(String::new(), |n| n.to_string())),
            old_style,
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:>4}", new.map_or(String::new(), |n| n.to_string())),
            new_style,
        ),
        Span::raw(" "),
    ];
    spans.extend(line.spans);
    Line::from(spans)
}

fn render_full_side_by_side_line(
    line: &str,
    width: usize,
    syntax: Option<Syntax>,
) -> Line<'static> {
    let text = truncate_chars(line, width);
    static_line(highlight_diff_line_for_syntax(&text, syntax))
}

fn render_side_by_side_row(
    old: Option<&SideDiffCell>,
    new: Option<&SideDiffCell>,
    width: usize,
    syntax: Option<Syntax>,
) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }

    let separator_width = SIDE_SEPARATOR.chars().count();
    let body_width = width.saturating_sub(separator_width);
    let old_width = body_width / 2;
    let new_width = body_width.saturating_sub(old_width);

    let mut spans = Vec::new();
    spans.extend(render_side_cell(old, old_width, syntax));
    if width >= separator_width {
        spans.push(Span::styled(
            SIDE_SEPARATOR,
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans.extend(render_side_cell(new, new_width, syntax));
    Line::from(spans)
}

fn render_side_cell(
    cell: Option<&SideDiffCell>,
    width: usize,
    syntax: Option<Syntax>,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let Some(cell) = cell else {
        return vec![Span::raw(" ".repeat(width))];
    };

    let base_style = diff_content_style(cell.kind);
    let number_style = diff_number_style(cell.kind);
    let marker_style = diff_marker_style(cell.kind);
    let marker = match cell.kind {
        DiffContentKind::Context => " ",
        DiffContentKind::Added => "+",
        DiffContentKind::Removed => "-",
    };

    let mut remaining = width;
    let mut spans = Vec::new();
    push_capped_span(
        &mut spans,
        &format!("{:>width$}", cell.number, width = SIDE_NUMBER_WIDTH),
        number_style,
        &mut remaining,
    );
    push_capped_span(&mut spans, " ", base_style, &mut remaining);
    push_capped_span(&mut spans, marker, marker_style, &mut remaining);
    push_capped_span(&mut spans, " ", base_style, &mut remaining);

    if remaining > 0 {
        let text = truncate_chars(&cell.text, remaining);
        spans.extend(highlight_code(&text, syntax, base_style));
        let used = spans_width(&spans).min(width);
        remaining = width.saturating_sub(used);
    }
    if remaining > 0 {
        spans.push(Span::styled(" ".repeat(remaining), base_style));
    }
    spans
}

fn diff_content_style(kind: DiffContentKind) -> Style {
    match kind {
        DiffContentKind::Context => Style::default(),
        DiffContentKind::Added => Style::default().fg(Color::Gray).bg(DIFF_ADDED_BG),
        DiffContentKind::Removed => Style::default().fg(Color::Gray).bg(DIFF_REMOVED_BG),
    }
}

fn diff_number_style(kind: DiffContentKind) -> Style {
    match kind {
        DiffContentKind::Context => Style::default().fg(Color::DarkGray),
        DiffContentKind::Added => Style::default().fg(Color::LightGreen).bg(DIFF_ADDED_BG),
        DiffContentKind::Removed => Style::default().fg(Color::LightRed).bg(DIFF_REMOVED_BG),
    }
}

fn diff_marker_style(kind: DiffContentKind) -> Style {
    match kind {
        DiffContentKind::Context => Style::default(),
        DiffContentKind::Added => Style::default()
            .fg(Color::Green)
            .bg(DIFF_ADDED_BG)
            .add_modifier(Modifier::BOLD),
        DiffContentKind::Removed => Style::default()
            .fg(Color::Red)
            .bg(DIFF_REMOVED_BG)
            .add_modifier(Modifier::BOLD),
    }
}

fn push_capped_span(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    style: Style,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    let text = truncate_chars(text, *remaining);
    *remaining = (*remaining).saturating_sub(text.chars().count());
    spans.push(Span::styled(text, style));
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| span.content.as_ref().chars().count())
        .sum()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn static_line(line: Line<'_>) -> Line<'static> {
    Line::from(
        line.spans
            .into_iter()
            .map(|span| Span::styled(span.content.into_owned(), span.style))
            .collect::<Vec<_>>(),
    )
}

fn diff_line_syntax(line: &str) -> Option<Syntax> {
    if let Some(path) = line.strip_prefix("+++ b/") {
        return path_syntax(path);
    }
    if let Some(path) = line.strip_prefix("diff --git ") {
        let path = path.split_whitespace().nth(1)?.strip_prefix("b/")?;
        return path_syntax(path);
    }
    None
}

fn path_syntax(path: &str) -> Option<Syntax> {
    if path.ends_with(".rs") {
        Some(Syntax::Rust)
    } else if path.ends_with(".kt") || path.ends_with(".kts") {
        Some(Syntax::Kotlin)
    } else {
        None
    }
}

fn highlight_code(code: &str, syntax: Option<Syntax>, default_style: Style) -> Vec<Span<'static>> {
    let Some(syntax) = syntax else {
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
    keyword.then_some(color_style(Color::Yellow, base).add_modifier(Modifier::BOLD))
}
