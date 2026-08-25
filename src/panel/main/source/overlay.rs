//! Laying a hunk's added and removed lines over the file they came from.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::ui;

use super::*;

pub(super) struct SourceRenderOptions {
    pub(super) show_path: bool,
    pub(super) side_by_side: bool,
    pub(super) viewport_width: u16,
}

pub(super) fn full_source_with_inline_diff(
    cache: &mut RenderCache,
    path: &str,
    body: &[String],
    notes: &BTreeMap<usize, Vec<String>>,
    indent: &str,
    options: SourceRenderOptions,
) -> Option<Vec<Line<'static>>> {
    // Cache file reads across the multiple times we're invoked per frame.
    let text = cache.read(path)?.to_owned();
    let overlay = inline_diff_overlay(body);
    let label = if options.show_path {
        format!("source {path}")
    } else {
        "source".to_string()
    };
    let mut lines = vec![section_header(indent, &label)];
    if options.side_by_side {
        lines.extend(side_by_side_inline_source_lines(
            path,
            &text,
            &overlay,
            notes,
            indent,
            options.viewport_width,
        ));
        return Some(lines);
    }

    let mut total_lines = 0usize;
    for (idx, source) in text.lines().enumerate() {
        total_lines = idx + 1;
        let line_no = idx + 1;
        push_source_notes(&mut lines, indent, notes.get(&line_no).map(Vec::as_slice));
        if let Some(removed) = overlay.removed_before.get(&line_no) {
            for removed_line in removed {
                lines.push(source_line(
                    path,
                    indent,
                    removed_line.old_line,
                    '-',
                    &removed_line.text,
                ));
            }
        }
        let marker = if overlay.added_lines.contains(&line_no) {
            '+'
        } else {
            '|'
        };
        lines.push(source_line(path, indent, Some(line_no), marker, source));
    }

    let eof_line = total_lines + 1;
    if let Some(removed) = overlay.removed_before.get(&eof_line) {
        for removed_line in removed {
            lines.push(source_line(
                path,
                indent,
                removed_line.old_line,
                '-',
                &removed_line.text,
            ));
        }
    }
    for (_, line_notes) in notes.range(eof_line..) {
        push_source_notes(&mut lines, indent, Some(line_notes.as_slice()));
    }

    Some(lines)
}

struct InlineSourceCell<'a> {
    line_no: Option<usize>,
    marker: char,
    source: &'a str,
}

fn side_by_side_inline_source_lines(
    path: &str,
    text: &str,
    overlay: &InlineDiffOverlay,
    notes: &BTreeMap<usize, Vec<String>>,
    indent: &str,
    viewport_width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut pending_removed = VecDeque::<&RemovedSourceLine>::new();
    let mut total_lines = 0usize;

    for (idx, source) in text.lines().enumerate() {
        total_lines = idx + 1;
        let line_no = idx + 1;
        push_source_notes(&mut lines, indent, notes.get(&line_no).map(Vec::as_slice));
        if let Some(removed) = overlay.removed_before.get(&line_no) {
            pending_removed.extend(removed);
        }

        if overlay.added_lines.contains(&line_no) {
            let old = pending_removed
                .pop_front()
                .map(|removed| removed_inline_cell(removed));
            let new = InlineSourceCell {
                line_no: Some(line_no),
                marker: '+',
                source,
            };
            lines.push(side_by_side_source_line(
                path,
                indent,
                old.as_ref(),
                Some(&new),
                viewport_width,
            ));
        } else {
            flush_removed_source_lines(
                &mut lines,
                path,
                indent,
                &mut pending_removed,
                viewport_width,
            );
            let old_line = overlay
                .old_line_for_new
                .get(&line_no)
                .copied()
                .or(Some(line_no));
            let old = InlineSourceCell {
                line_no: old_line,
                marker: '|',
                source,
            };
            let new = InlineSourceCell {
                line_no: Some(line_no),
                marker: '|',
                source,
            };
            lines.push(side_by_side_source_line(
                path,
                indent,
                Some(&old),
                Some(&new),
                viewport_width,
            ));
        }
    }

    let eof_line = total_lines + 1;
    if let Some(removed) = overlay.removed_before.get(&eof_line) {
        pending_removed.extend(removed);
    }
    flush_removed_source_lines(
        &mut lines,
        path,
        indent,
        &mut pending_removed,
        viewport_width,
    );
    for (_, line_notes) in notes.range(eof_line..) {
        push_source_notes(&mut lines, indent, Some(line_notes.as_slice()));
    }

    lines
}

fn flush_removed_source_lines(
    lines: &mut Vec<Line<'static>>,
    path: &str,
    indent: &str,
    pending_removed: &mut VecDeque<&RemovedSourceLine>,
    viewport_width: u16,
) {
    while let Some(removed) = pending_removed.pop_front() {
        let old = removed_inline_cell(removed);
        lines.push(side_by_side_source_line(
            path,
            indent,
            Some(&old),
            None,
            viewport_width,
        ));
    }
}

fn removed_inline_cell(removed: &RemovedSourceLine) -> InlineSourceCell<'_> {
    InlineSourceCell {
        line_no: removed.old_line,
        marker: '-',
        source: &removed.text,
    }
}

fn side_by_side_source_line(
    path: &str,
    indent: &str,
    old: Option<&InlineSourceCell<'_>>,
    new: Option<&InlineSourceCell<'_>>,
    viewport_width: u16,
) -> Line<'static> {
    let mut spans = context_prefix(indent);
    let prefix_width = context_prefix_width(indent);
    let width = (viewport_width as usize).saturating_sub(prefix_width);
    if width == 0 {
        return Line::from(spans);
    }
    let separator_width = SIDE_SEPARATOR.chars().count();
    let body_width = width.saturating_sub(separator_width);
    let old_width = body_width / 2;
    let new_width = body_width.saturating_sub(old_width);

    spans.extend(side_source_cell_spans(path, old, old_width));
    if width >= separator_width {
        spans.push(Span::styled(
            SIDE_SEPARATOR,
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans.extend(side_source_cell_spans(path, new, new_width));
    Line::from(spans)
}

fn side_source_cell_spans(
    path: &str,
    cell: Option<&InlineSourceCell<'_>>,
    width: usize,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let Some(cell) = cell else {
        return vec![Span::raw(" ".repeat(width))];
    };

    let (line_style, marker_style, bg) = source_marker_styles(cell.marker);
    let base_style = bg.map_or(Style::default(), |bg| Style::default().bg(bg));
    let mut remaining = width;
    let mut spans = Vec::new();
    push_capped_span(
        &mut spans,
        &format!(
            "{:>5}",
            cell.line_no.map_or(String::new(), |n| n.to_string())
        ),
        line_style,
        &mut remaining,
    );
    push_capped_span(&mut spans, " ", line_style, &mut remaining);
    push_capped_span(
        &mut spans,
        &format!("{} ", cell.marker),
        marker_style,
        &mut remaining,
    );
    if remaining > 0 {
        let source = truncate_chars(cell.source, remaining);
        spans.extend(apply_optional_bg(
            owned_spans(ui::highlight_source_line_for_path(&source, path)),
            bg,
        ));
        remaining = width.saturating_sub(spans_width(&spans).min(width));
    }
    if remaining > 0 {
        spans.push(Span::styled(" ".repeat(remaining), base_style));
    }
    spans
}

fn source_marker_styles(marker: char) -> (Style, Style, Option<Color>) {
    match marker {
        '+' => (
            Style::default().fg(Color::LightGreen).bg(INLINE_ADDED_BG),
            Style::default()
                .fg(Color::Green)
                .bg(INLINE_ADDED_BG)
                .add_modifier(Modifier::BOLD),
            Some(INLINE_ADDED_BG),
        ),
        '-' => (
            Style::default().fg(Color::LightRed).bg(INLINE_REMOVED_BG),
            Style::default()
                .fg(Color::Red)
                .bg(INLINE_REMOVED_BG)
                .add_modifier(Modifier::BOLD),
            Some(INLINE_REMOVED_BG),
        ),
        _ => (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
            None,
        ),
    }
}

#[derive(Default)]
pub(in crate::panel::main) struct InlineDiffOverlay {
    pub removed_before: BTreeMap<usize, Vec<RemovedSourceLine>>,
    pub added_lines: BTreeSet<usize>,
    pub old_line_for_new: BTreeMap<usize, usize>,
}

pub(in crate::panel::main) struct RemovedSourceLine {
    pub old_line: Option<usize>,
    pub text: String,
}

pub(in crate::panel::main) fn inline_diff_overlay(body: &[String]) -> InlineDiffOverlay {
    let mut overlay = InlineDiffOverlay::default();
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut in_hunk = false;

    for line in body {
        if let Some((old_start, new_start)) = parse_hunk_header(line) {
            old_line = old_start;
            new_line = new_start;
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        if line.starts_with("\\ No newline") {
            continue;
        }
        if let Some(source) = line.strip_prefix(' ') {
            overlay.old_line_for_new.insert(new_line.max(1), old_line);
            old_line = old_line.saturating_add(1);
            new_line = new_line.saturating_add(1);
            let _ = source;
        } else if let Some(source) = line.strip_prefix('-') {
            overlay
                .removed_before
                .entry(new_line.max(1))
                .or_default()
                .push(RemovedSourceLine {
                    old_line: Some(old_line),
                    text: source.to_string(),
                });
            old_line = old_line.saturating_add(1);
        } else if line.starts_with('+') {
            overlay.added_lines.insert(new_line.max(1));
            new_line = new_line.saturating_add(1);
        }
    }

    overlay
}

pub(super) fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ ")?;
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    Some((parse_hunk_start(old)?, parse_hunk_start(new)?))
}

fn parse_hunk_start(part: &str) -> Option<usize> {
    part.split(',').next()?.parse().ok()
}
