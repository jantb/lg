//! Building one rendered line: its prefix, its spans, and its note styling.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::ui;

pub(super) const STYLE_WARN_NOTE_BG: Color = Color::Rgb(92, 68, 18);

pub(super) const STYLE_FAIL_NOTE_BG: Color = Color::Rgb(88, 24, 30);

pub(super) const STYLE_WARN_LABEL_BG: Color = Color::Yellow;

pub(super) const STYLE_FAIL_LABEL_BG: Color = Color::Red;

pub(super) const INLINE_ADDED_BG: Color = Color::Rgb(24, 54, 34);

pub(super) const INLINE_REMOVED_BG: Color = Color::Rgb(60, 28, 38);

pub(super) const SIDE_SEPARATOR: &str = " | ";

pub(super) fn section_header(indent: &str, label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("{indent}  │ {label}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

pub(super) fn source_line(
    path: &str,
    indent: &str,
    line_no: Option<usize>,
    marker: char,
    source: &str,
) -> Line<'static> {
    let mut spans = context_prefix(indent);
    let (line_style, marker_style, bg) = match marker {
        '+' => (
            Style::default()
                .fg(Color::LightGreen)
                .bg(Color::Rgb(24, 54, 34)),
            Style::default()
                .fg(Color::Green)
                .bg(Color::Rgb(24, 54, 34))
                .add_modifier(Modifier::BOLD),
            Some(Color::Rgb(24, 54, 34)),
        ),
        '-' => (
            Style::default()
                .fg(Color::LightRed)
                .bg(Color::Rgb(60, 28, 38)),
            Style::default()
                .fg(Color::Red)
                .bg(Color::Rgb(60, 28, 38))
                .add_modifier(Modifier::BOLD),
            Some(Color::Rgb(60, 28, 38)),
        ),
        _ => (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
            None,
        ),
    };
    spans.push(Span::styled(
        format!("{:>5} ", line_no.map_or(String::new(), |n| n.to_string())),
        line_style,
    ));
    spans.push(Span::styled(format!("{marker} "), marker_style));
    let code = ui::highlight_source_line_for_path(source, path);
    spans.extend(apply_optional_bg(owned_spans(code), bg));
    Line::from(spans)
}

pub(super) fn push_source_notes(
    lines: &mut Vec<Line<'static>>,
    indent: &str,
    notes: Option<&[String]>,
) {
    let Some(notes) = notes else {
        return;
    };
    for note in notes {
        lines.push(source_note_line(indent, note));
    }
}

pub(super) fn source_note_line(indent: &str, note: &str) -> Line<'static> {
    if let Some((severity, reason)) = style_note_parts(note) {
        return style_note_line(indent, severity, reason);
    }
    if let Some(reason) = note.strip_prefix("entry point: ") {
        return entry_point_note_line(indent, reason);
    }
    let mut spans = context_prefix(indent);
    spans.push(Span::styled(
        "      · ".to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(
        "review note: ".to_string(),
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        note.to_string(),
        Style::default().fg(Color::Gray),
    ));
    Line::from(spans)
}

pub(super) fn entry_point_note_line(indent: &str, reason: &str) -> Line<'static> {
    let mut spans = context_prefix(indent);
    spans.push(Span::styled(
        "      ◆ ".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        " ENTRY POINT ".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" {reason}"),
        Style::default()
            .fg(Color::LightCyan)
            .bg(Color::Rgb(18, 50, 58))
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

pub(super) fn style_note_parts(note: &str) -> Option<(crate::state::ReviewStyleSeverity, &str)> {
    note.strip_prefix("style warn: ")
        .map(|reason| (crate::state::ReviewStyleSeverity::Warn, reason))
        .or_else(|| {
            note.strip_prefix("style fail: ")
                .map(|reason| (crate::state::ReviewStyleSeverity::Fail, reason))
        })
}

pub(super) fn style_note_line(
    indent: &str,
    severity: crate::state::ReviewStyleSeverity,
    reason: &str,
) -> Line<'static> {
    let (label_bg, note_bg, note_fg) = match severity {
        crate::state::ReviewStyleSeverity::Ok => {
            (Color::Green, Color::Rgb(24, 54, 34), Color::LightGreen)
        }
        crate::state::ReviewStyleSeverity::Warn => {
            (STYLE_WARN_LABEL_BG, STYLE_WARN_NOTE_BG, Color::LightYellow)
        }
        crate::state::ReviewStyleSeverity::Fail => {
            (STYLE_FAIL_LABEL_BG, STYLE_FAIL_NOTE_BG, Color::White)
        }
    };
    let mut spans = context_prefix(indent);
    spans.push(Span::styled(
        "      ! ".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(label_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" STYLE {} ", severity.label()),
        Style::default()
            .fg(Color::Black)
            .bg(label_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" {reason}"),
        Style::default()
            .fg(note_fg)
            .bg(note_bg)
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

pub(super) fn apply_optional_bg(
    spans: Vec<Span<'static>>,
    bg: Option<Color>,
) -> Vec<Span<'static>> {
    let Some(bg) = bg else {
        return spans;
    };
    spans
        .into_iter()
        .map(|span| Span::styled(span.content, span.style.bg(bg)))
        .collect()
}

pub(super) fn context_prefix(indent: &str) -> Vec<Span<'static>> {
    vec![Span::styled(
        format!("{indent}  │ "),
        Style::default().fg(Color::DarkGray),
    )]
}

pub(super) fn context_prefix_width(indent: &str) -> usize {
    indent.chars().count() + "  │ ".chars().count()
}

pub(super) fn prefixed_side_by_side_diff_lines(
    body: &[String],
    syntax_path: Option<&str>,
    indent: &str,
    viewport_width: u16,
) -> Vec<Line<'static>> {
    let prefix_width = context_prefix_width(indent);
    let width = (viewport_width as usize)
        .saturating_sub(prefix_width)
        .min(u16::MAX as usize) as u16;
    let text = body.join("\n");
    let diff_lines = syntax_path
        .map(|path| ui::highlight_side_by_side_diff_text_for_path(&text, width, path))
        .unwrap_or_else(|| ui::highlight_side_by_side_diff_text(&text, width));
    diff_lines
        .into_iter()
        .map(|line| {
            let mut spans = context_prefix(indent);
            spans.extend(owned_spans(line));
            Line::from(spans)
        })
        .collect()
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

pub(super) fn source_context_line(
    context: &str,
    syntax_path: Option<&str>,
    indent: &str,
) -> Line<'static> {
    let mut spans = context_prefix(indent);
    let Some((line_no, source)) = context.split_once(" | ") else {
        spans.push(Span::styled(
            context.to_string(),
            Style::default().fg(Color::Gray),
        ));
        return Line::from(spans);
    };

    spans.push(Span::styled(
        format!("{line_no} "),
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled("| ", Style::default().fg(Color::DarkGray)));
    if let Some(path) = syntax_path {
        spans.extend(owned_spans(ui::highlight_source_line_for_path(
            source, path,
        )));
    } else {
        spans.push(Span::styled(
            source.to_string(),
            Style::default().fg(Color::Gray),
        ));
    }
    Line::from(spans)
}

pub(in crate::panel::main) fn owned_spans(line: Line<'_>) -> Vec<Span<'static>> {
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect()
}
