use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ui;

pub(super) struct SourceSection {
    pub path: String,
    pub body: Vec<String>,
    pub context: Vec<String>,
}

/// Per-frame cache so file reads aren't repeated across the
/// scroll-bound and rendering passes of the review panel.
#[derive(Default)]
pub(super) struct RenderCache {
    files: HashMap<String, Option<String>>,
}

impl RenderCache {
    fn read(&mut self, path: &str) -> Option<&str> {
        if !self.files.contains_key(path) {
            self.files
                .insert(path.to_string(), std::fs::read_to_string(path).ok());
        }
        self.files.get(path).and_then(|opt| opt.as_deref())
    }
}

pub(super) fn review_source_context_lines(
    cache: &mut RenderCache,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    syntax_path: Option<&str>,
    indent: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![section_header(indent, "source context")];
    let sections = source_sections(review, node);
    if !sections.is_empty() {
        let multiple = sections.len() > 1;
        for section in sections {
            if let Some(mut source) =
                full_source_with_inline_diff(cache, &section.path, &section.body, indent, multiple)
            {
                lines.append(&mut source);
            } else {
                lines.extend(fallback_source_context_lines(
                    &section.body,
                    &section.context,
                    Some(&section.path),
                    indent,
                ));
            }
        }
        return lines;
    }

    lines.extend(fallback_source_context_lines(
        &node.body,
        &node.context,
        syntax_path,
        indent,
    ));
    lines
}

pub(super) fn source_sections(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> Vec<SourceSection> {
    let mut sections = Vec::new();
    let mut seen_paths = BTreeSet::new();
    for candidate in std::iter::once(node).chain(
        review
            .nodes
            .iter()
            .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
    ) {
        let Some(path) = super::review::review_node_path(&candidate.title) else {
            continue;
        };
        if candidate.body.is_empty() && candidate.context.is_empty() {
            continue;
        }
        if !seen_paths.insert(path.to_string()) {
            continue;
        }
        sections.push(SourceSection {
            path: path.to_string(),
            body: candidate.body.clone(),
            context: candidate.context.clone(),
        });
    }
    sections
}

fn review_node_in_subtree(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    root_id: &str,
) -> bool {
    let mut parent = node.parent.as_deref();
    while let Some(parent_id) = parent {
        if parent_id == root_id {
            return true;
        }
        parent = review
            .nodes
            .iter()
            .find(|candidate| candidate.id == parent_id)
            .and_then(|candidate| candidate.parent.as_deref());
    }
    false
}

fn fallback_source_context_lines(
    body: &[String],
    context: &[String],
    syntax_path: Option<&str>,
    indent: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !body.is_empty() {
        lines.push(section_header(indent, "diff"));
        for body in body {
            let mut spans = context_prefix(indent);
            let body_line = syntax_path
                .map(|path| ui::highlight_diff_line_for_path(body, path))
                .unwrap_or_else(|| ui::highlight_diff_line(body));
            spans.extend(owned_spans(body_line));
            lines.push(Line::from(spans));
        }
    }
    if !context.is_empty() {
        lines.push(section_header(indent, "source"));
        for context in context {
            lines.push(source_context_line(context, syntax_path, indent));
        }
    }
    lines
}

fn full_source_with_inline_diff(
    cache: &mut RenderCache,
    path: &str,
    body: &[String],
    indent: &str,
    show_path: bool,
) -> Option<Vec<Line<'static>>> {
    // Cache file reads across the multiple times we're invoked per frame.
    let text = cache.read(path)?.to_owned();
    let overlay = inline_diff_overlay(body);
    let label = if show_path {
        format!("source {path}")
    } else {
        "source".to_string()
    };
    let mut lines = vec![section_header(indent, &label)];

    let mut total_lines = 0usize;
    for (idx, source) in text.lines().enumerate() {
        total_lines = idx + 1;
        let line_no = idx + 1;
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

    Some(lines)
}

#[derive(Default)]
pub(super) struct InlineDiffOverlay {
    pub removed_before: BTreeMap<usize, Vec<RemovedSourceLine>>,
    pub added_lines: BTreeSet<usize>,
}

pub(super) struct RemovedSourceLine {
    pub old_line: Option<usize>,
    pub text: String,
}

pub(super) fn inline_diff_overlay(body: &[String]) -> InlineDiffOverlay {
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

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ ")?;
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    Some((parse_hunk_start(old)?, parse_hunk_start(new)?))
}

fn parse_hunk_start(part: &str) -> Option<usize> {
    part.split(',').next()?.parse().ok()
}

fn section_header(indent: &str, label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("{indent}  │ {label}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn source_line(
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

fn apply_optional_bg(spans: Vec<Span<'static>>, bg: Option<Color>) -> Vec<Span<'static>> {
    let Some(bg) = bg else {
        return spans;
    };
    spans
        .into_iter()
        .map(|span| Span::styled(span.content, span.style.bg(bg)))
        .collect()
}

fn context_prefix(indent: &str) -> Vec<Span<'static>> {
    vec![Span::styled(
        format!("{indent}  │ "),
        Style::default().fg(Color::DarkGray),
    )]
}

fn source_context_line(context: &str, syntax_path: Option<&str>, indent: &str) -> Line<'static> {
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

pub(super) fn owned_spans(line: Line<'_>) -> Vec<Span<'static>> {
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect()
}
