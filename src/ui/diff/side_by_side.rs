//! Laying a diff out in two columns with line numbers, and wrapping rows to the pane.

use super::*;

pub(super) const DIFF_ADDED_BG: Color = Color::Rgb(24, 54, 34);
pub(super) const DIFF_REMOVED_BG: Color = Color::Rgb(60, 28, 38);
pub(super) const SIDE_SEPARATOR: &str = " | ";
pub(super) const SIDE_NUMBER_WIDTH: usize = 4;
/// Number, space, +/- marker, space — what a side-by-side cell spends before
/// the code starts, and what a wrapped continuation row leaves blank.
pub(super) const SIDE_GUTTER_WIDTH: usize = SIDE_NUMBER_WIDTH + 3;
/// The `old new ` prefix [`add_diff_line_numbers`] puts on a unified diff line.
pub(super) const DIFF_NUMBER_GUTTER: usize = SIDE_NUMBER_WIDTH * 2 + 2;

#[derive(Clone, Copy)]
pub(super) struct DiffLineNumbers {
    pub(super) old_line: u32,
    pub(super) new_line: u32,
}

#[derive(Clone, Copy)]
pub(super) enum DiffContentKind {
    Context,
    Added,
    Removed,
}

pub(super) struct SideDiffCell {
    pub(super) number: u32,
    pub(super) text: String,
    pub(super) kind: DiffContentKind,
}

pub(super) struct SideBySideDiffRenderer {
    pub(super) width: usize,
    pub(super) syntax: Option<Syntax>,
    pub(super) numbers: Option<DiffLineNumbers>,
    pub(super) in_hunk: bool,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) pending_removed: Vec<SideDiffCell>,
    pub(super) pending_added: Vec<SideDiffCell>,
}

impl SideBySideDiffRenderer {
    pub(super) fn new(width: usize) -> Self {
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

    pub(super) fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_change_run();
        self.lines
    }

    pub(super) fn push_line(&mut self, line: &str) {
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
            && let Some(mut numbers) = self.numbers
        {
            match kind {
                DiffContentKind::Context => {
                    self.flush_change_run();
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
                    let number = numbers.old_line;
                    numbers.old_line = numbers.old_line.saturating_add(1);
                    self.pending_removed.push(SideDiffCell {
                        number,
                        text: line.strip_prefix('-').unwrap_or(line).to_string(),
                        kind,
                    });
                }
            }
            self.numbers = Some(numbers);
            return;
        }

        self.push_full_line(line);
    }

    pub(super) fn push_full_line(&mut self, line: &str) {
        self.flush_change_run();
        self.lines.extend(render_full_side_by_side_lines(
            line,
            self.width,
            self.syntax,
        ));
    }

    pub(super) fn flush_change_run(&mut self) {
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

pub(super) fn parse_hunk_line_numbers(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    Some((parse_hunk_start(old)?, parse_hunk_start(new)?))
}

pub(super) fn parse_hunk_start(part: &str) -> Option<u32> {
    part.split(',').next()?.parse().ok()
}

pub(super) fn diff_content_kind(line: &str) -> Option<DiffContentKind> {
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

pub(super) fn add_diff_line_numbers<'a>(
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

pub(super) fn render_full_side_by_side_lines(
    line: &str,
    width: usize,
    syntax: Option<Syntax>,
) -> Vec<Line<'static>> {
    wrap_line(highlight_diff_line_for_syntax(line, syntax), width, 0)
}

/// One side-by-side pair. Either cell can wrap into several rows; the pair
/// takes as many rows as the longer of the two, with the shorter side padded so
/// the change block keeps its background.
pub(super) fn render_side_by_side_rows(
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
pub(super) fn render_side_cell_rows(
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
pub(super) fn render_side_cell_gutter(cell: &SideDiffCell, width: usize) -> Vec<Span<'static>> {
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
pub(super) fn side_cell_filler(cell: Option<&SideDiffCell>, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let style = cell.map_or_else(Style::default, |cell| diff_content_style(cell.kind));
    vec![Span::styled(" ".repeat(width), style)]
}

pub(super) fn diff_content_style(kind: DiffContentKind) -> Style {
    match kind {
        DiffContentKind::Context => Style::default(),
        DiffContentKind::Added => Style::default().fg(Color::Gray).bg(DIFF_ADDED_BG),
        DiffContentKind::Removed => Style::default().fg(Color::Gray).bg(DIFF_REMOVED_BG),
    }
}

pub(super) fn diff_number_style(kind: DiffContentKind) -> Style {
    match kind {
        DiffContentKind::Context => Style::default().fg(Color::DarkGray),
        DiffContentKind::Added => Style::default().fg(Color::LightGreen).bg(DIFF_ADDED_BG),
        DiffContentKind::Removed => Style::default().fg(Color::LightRed).bg(DIFF_REMOVED_BG),
    }
}

pub(super) fn diff_marker_style(kind: DiffContentKind) -> Style {
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

pub(super) fn push_capped_span(
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

pub(super) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| span.content.as_ref().chars().count())
        .sum()
}

pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub(super) fn owned_spans(line: Line<'_>) -> Vec<Span<'static>> {
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect()
}

/// Wrap one rendered line to `width`, treating its first `gutter` columns as a
/// prefix that continuation rows leave blank, so wrapped text stays under the
/// text it continues rather than under the line numbers.
pub(super) fn wrap_line(line: Line<'_>, width: usize, gutter: usize) -> Vec<Line<'static>> {
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
pub(super) fn wrapped_row_count(total: usize, gutter: usize, width: usize) -> usize {
    if width == 0 || gutter >= width || total <= width {
        return 1;
    }
    total.saturating_sub(gutter).div_ceil(width - gutter).max(1)
}

/// Split spans at a column, cutting the span that straddles it in two.
pub(super) fn split_spans_at(
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
pub(super) fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
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

pub(super) fn split_text_at(text: &str, column: usize) -> (String, String) {
    let split = text
        .char_indices()
        .nth(column)
        .map_or(text.len(), |(idx, _)| idx);
    (text[..split].to_string(), text[split..].to_string())
}
