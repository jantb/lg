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

const DIFF_ADDED_BG: Color = Color::Rgb(24, 54, 34);
const DIFF_REMOVED_BG: Color = Color::Rgb(60, 28, 38);
const SIDE_SEPARATOR: &str = " | ";
const SIDE_NUMBER_WIDTH: usize = 4;
/// Number, space, +/- marker, space — what a side-by-side cell spends before
/// the code starts, and what a wrapped continuation row leaves blank.
const SIDE_GUTTER_WIDTH: usize = SIDE_NUMBER_WIDTH + 3;
/// The `old new ` prefix [`add_diff_line_numbers`] puts on a unified diff line.
const DIFF_NUMBER_GUTTER: usize = SIDE_NUMBER_WIDTH * 2 + 2;

#[derive(Clone, Copy)]
enum Syntax {
    CSharp,
    Kotlin,
    Markdown,
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
                    self.lines.extend(render_side_by_side_rows(
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
        self.lines.extend(render_full_side_by_side_lines(
            line,
            self.width,
            self.syntax,
        ));
    }

    fn flush_change_run(&mut self) {
        let rows = self.pending_removed.len().max(self.pending_added.len());
        for idx in 0..rows {
            self.lines.extend(render_side_by_side_rows(
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

fn render_full_side_by_side_lines(
    line: &str,
    width: usize,
    syntax: Option<Syntax>,
) -> Vec<Line<'static>> {
    wrap_line(highlight_diff_line_for_syntax(line, syntax), width, 0)
}

/// One side-by-side pair. Either cell can wrap into several rows; the pair
/// takes as many rows as the longer of the two, with the shorter side padded so
/// the change block keeps its background.
fn render_side_by_side_rows(
    old: Option<&SideDiffCell>,
    new: Option<&SideDiffCell>,
    width: usize,
    syntax: Option<Syntax>,
) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::from("")];
    }

    let separator_width = SIDE_SEPARATOR.chars().count();
    let body_width = width.saturating_sub(separator_width);
    let old_width = body_width / 2;
    let new_width = body_width.saturating_sub(old_width);

    let old_rows = render_side_cell_rows(old, old_width, syntax);
    let new_rows = render_side_cell_rows(new, new_width, syntax);
    let rows = old_rows.len().max(new_rows.len()).max(1);

    (0..rows)
        .map(|idx| {
            let mut spans = Vec::new();
            spans.extend(
                old_rows
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| side_cell_filler(old, old_width)),
            );
            if width >= separator_width {
                spans.push(Span::styled(
                    SIDE_SEPARATOR,
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.extend(
                new_rows
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| side_cell_filler(new, new_width)),
            );
            Line::from(spans)
        })
        .collect()
}

/// A cell's rows, each padded to exactly `width` so the two columns line up.
fn render_side_cell_rows(
    cell: Option<&SideDiffCell>,
    width: usize,
    syntax: Option<Syntax>,
) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![Vec::new()];
    }

    let Some(cell) = cell else {
        return vec![vec![Span::raw(" ".repeat(width))]];
    };

    let base_style = diff_content_style(cell.kind);
    if width <= SIDE_GUTTER_WIDTH {
        // No room for code beside the gutter — nothing useful to wrap into.
        return vec![render_side_cell_gutter(cell, width)];
    }

    let content_width = width - SIDE_GUTTER_WIDTH;
    let content = highlight_code(&cell.text, syntax, base_style);
    wrap_spans(content, content_width)
        .into_iter()
        .enumerate()
        .map(|(idx, content)| {
            let mut spans = if idx == 0 {
                render_side_cell_gutter(cell, SIDE_GUTTER_WIDTH)
            } else {
                vec![Span::styled(" ".repeat(SIDE_GUTTER_WIDTH), base_style)]
            };
            spans.extend(content);
            let remaining = width.saturating_sub(spans_width(&spans).min(width));
            if remaining > 0 {
                spans.push(Span::styled(" ".repeat(remaining), base_style));
            }
            spans
        })
        .collect()
}

/// Line number, marker, and the spaces around them, capped at `width`.
fn render_side_cell_gutter(cell: &SideDiffCell, width: usize) -> Vec<Span<'static>> {
    let base_style = diff_content_style(cell.kind);
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
        diff_number_style(cell.kind),
        &mut remaining,
    );
    push_capped_span(&mut spans, " ", base_style, &mut remaining);
    push_capped_span(
        &mut spans,
        marker,
        diff_marker_style(cell.kind),
        &mut remaining,
    );
    push_capped_span(&mut spans, " ", base_style, &mut remaining);
    if remaining > 0 {
        spans.push(Span::styled(" ".repeat(remaining), base_style));
    }
    spans
}

/// The blank a cell shows on rows the *other* side wrapped into. A cell that
/// exists keeps its background so the change block stays solid.
fn side_cell_filler(cell: Option<&SideDiffCell>, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let style = cell.map_or_else(Style::default, |cell| diff_content_style(cell.kind));
    vec![Span::styled(" ".repeat(width), style)]
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

fn owned_spans(line: Line<'_>) -> Vec<Span<'static>> {
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect()
}

/// Wrap one rendered line to `width`, treating its first `gutter` columns as a
/// prefix that continuation rows leave blank, so wrapped text stays under the
/// text it continues rather than under the line numbers.
fn wrap_line(line: Line<'_>, width: usize, gutter: usize) -> Vec<Line<'static>> {
    let spans = owned_spans(line);
    if width == 0 || gutter >= width || spans_width(&spans) <= width {
        return vec![Line::from(spans)];
    }

    let (head, body) = split_spans_at(spans, gutter);
    // Carry only the background across: a wrapped `+` line should keep its
    // green block, not repeat the line number's colour in the blank gutter.
    let indent_style = body
        .first()
        .and_then(|span| span.style.bg)
        .map_or_else(Style::default, |bg| Style::default().bg(bg));
    let mut rows = wrap_spans(body, width - gutter).into_iter();
    let mut lines = Vec::new();
    if let Some(first) = rows.next() {
        let mut spans = head;
        spans.extend(first);
        lines.push(Line::from(spans));
    }
    for row in rows {
        let mut spans = Vec::new();
        if gutter > 0 {
            spans.push(Span::styled(" ".repeat(gutter), indent_style));
        }
        spans.extend(row);
        lines.push(Line::from(spans));
    }
    lines
}

/// How many rows [`wrap_line`] would produce for a line of `total` columns.
fn wrapped_row_count(total: usize, gutter: usize, width: usize) -> usize {
    if width == 0 || gutter >= width || total <= width {
        return 1;
    }
    total.saturating_sub(gutter).div_ceil(width - gutter).max(1)
}

/// Split spans at a column, cutting the span that straddles it in two.
fn split_spans_at(
    spans: Vec<Span<'static>>,
    column: usize,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let len = span.content.chars().count();
        if used >= column {
            tail.push(span);
        } else if used + len <= column {
            used += len;
            head.push(span);
        } else {
            let style = span.style;
            let (before, after) = split_text_at(span.content.as_ref(), column - used);
            head.push(Span::styled(before, style));
            tail.push(Span::styled(after, style));
            used = column;
        }
    }
    (head, tail)
}

/// Break spans into rows of at most `width` columns, keeping each span's style
/// across the break.
fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![spans];
    }

    let mut rows = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let style = span.style;
        let mut rest = span.content.into_owned();
        loop {
            let len = rest.chars().count();
            if used + len <= width {
                if len > 0 {
                    row.push(Span::styled(rest, style));
                    used += len;
                }
                break;
            }
            if used == width {
                rows.push(std::mem::take(&mut row));
                used = 0;
                continue;
            }
            let (head, tail) = split_text_at(&rest, width - used);
            row.push(Span::styled(head, style));
            used = width;
            rest = tail;
        }
    }
    rows.push(row);
    rows
}

fn split_text_at(text: &str, column: usize) -> (String, String) {
    let split = text
        .char_indices()
        .nth(column)
        .map_or(text.len(), |(idx, _)| idx);
    (text[..split].to_string(), text[split..].to_string())
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
    } else if path.ends_with(".md") || path.ends_with(".markdown") {
        Some(Syntax::Markdown)
    } else if path.ends_with(".cs") || path.ends_with(".csx") {
        Some(Syntax::CSharp)
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
    if matches!(syntax, Syntax::Markdown) {
        return highlight_markdown(code, default_style);
    }

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
        Syntax::CSharp => matches!(
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
        // Markdown never reaches the code lexer.
        Syntax::Markdown => false,
    };
    keyword.then_some(color_style(Color::Yellow, base).add_modifier(Modifier::BOLD))
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
