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
    pub notes: BTreeMap<usize, Vec<String>>,
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
            if let Some(mut source) = full_source_with_inline_diff(
                cache,
                &section.path,
                &section.body,
                &section.notes,
                indent,
                multiple,
            ) {
                lines.append(&mut source);
            } else {
                lines.extend(fallback_source_context_lines(
                    &section.body,
                    &section.context,
                    Some(&section.path),
                    &section.notes,
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
        &BTreeMap::new(),
        indent,
    ));
    lines
}

pub(super) fn source_sections(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> Vec<SourceSection> {
    if is_full_diff_root(node)
        && let Some(sections) = full_diff_source_sections(review)
        && !sections.is_empty()
    {
        let mut sections = sections;
        attach_inline_review_notes(review, node, &mut sections);
        return sections;
    }

    let mut sections = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut notes_by_path = inline_review_notes_by_path(review, node);
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
            notes: notes_by_path.remove(path).unwrap_or_default(),
        });
    }
    sections
}

fn is_full_diff_root(node: &crate::git::ReviewNode) -> bool {
    node.parent.is_none() && node.title == "Full diff against main"
}

fn full_diff_source_sections(review: &crate::git::AssistedReview) -> Option<Vec<SourceSection>> {
    let (_, diff) = review.report.split_once("\nFull diff against main\n")?;
    let mut sections = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_body = Vec::new();

    for line in diff.lines() {
        if let Some(path) = diff_git_path(line) {
            push_full_diff_section(&mut sections, current_path.take(), &mut current_body);
            current_path = Some(path);
        }
        if current_path.is_some() {
            current_body.push(line.to_string());
        }
    }
    push_full_diff_section(&mut sections, current_path, &mut current_body);

    Some(sections)
}

fn diff_git_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git a/")?;
    let (_, path) = rest.split_once(" b/")?;
    (path != "/dev/null" && !path.trim().is_empty()).then(|| path.trim().to_string())
}

fn push_full_diff_section(
    sections: &mut Vec<SourceSection>,
    path: Option<String>,
    body: &mut Vec<String>,
) {
    let Some(path) = path else {
        body.clear();
        return;
    };
    if body.is_empty() {
        return;
    }
    sections.push(SourceSection {
        path,
        body: std::mem::take(body),
        context: Vec::new(),
        notes: BTreeMap::new(),
    });
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

fn attach_inline_review_notes(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    sections: &mut [SourceSection],
) {
    let mut notes_by_path = inline_review_notes_by_path(review, node);
    for section in sections {
        section.notes = notes_by_path.remove(&section.path).unwrap_or_default();
    }
}

fn inline_review_notes_by_path(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> BTreeMap<String, BTreeMap<usize, Vec<String>>> {
    let mut notes: BTreeMap<String, BTreeMap<usize, Vec<String>>> = BTreeMap::new();
    for candidate in std::iter::once(node).chain(
        review
            .nodes
            .iter()
            .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
    ) {
        let Some(note) = inline_review_note_text(candidate) else {
            continue;
        };
        let Some(path) = super::review::review_node_path(&candidate.title) else {
            continue;
        };
        let Some(line) = review_node_line(&candidate.title)
            .or_else(|| first_body_hunk_new_line(&candidate.body))
            .map(|line| line.max(1))
        else {
            continue;
        };
        let line_notes = notes
            .entry(path.to_string())
            .or_default()
            .entry(line)
            .or_default();
        if !line_notes.contains(&note) {
            line_notes.push(note);
        }
    }
    notes
}

fn inline_review_note_text(node: &crate::git::ReviewNode) -> Option<String> {
    if let Some(effect) = node
        .body
        .iter()
        .find_map(|line| line.trim().strip_prefix("effect: "))
    {
        let effect = effect.trim();
        if !effect.is_empty() {
            return Some(effect.to_string());
        }
    }

    if node.id.contains(":hunk:")
        && let Some((_, description)) = node.title.split_once(" - ")
    {
        let description = description.trim();
        if !description.is_empty() {
            return Some(description.to_string());
        }
    }

    None
}

fn review_node_line(title: &str) -> Option<usize> {
    let location = title
        .split_once(" in ")
        .map(|(path, _)| path)
        .or_else(|| title.split_once(" - ").map(|(location, _)| location))
        .unwrap_or(title);
    let (_, line) = location.rsplit_once(':')?;
    line.chars()
        .all(|ch| ch.is_ascii_digit())
        .then(|| line.parse().ok())
        .flatten()
}

fn first_body_hunk_new_line(body: &[String]) -> Option<usize> {
    body.iter()
        .find_map(|line| parse_hunk_header(line).map(|(_, new_line)| new_line))
}

fn fallback_source_context_lines(
    body: &[String],
    context: &[String],
    syntax_path: Option<&str>,
    notes: &BTreeMap<usize, Vec<String>>,
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
    if !notes.is_empty() {
        lines.push(section_header(indent, "review notes"));
        for line_notes in notes.values() {
            for note in line_notes {
                lines.push(source_note_line(indent, note));
            }
        }
    }
    lines
}

fn full_source_with_inline_diff(
    cache: &mut RenderCache,
    path: &str,
    body: &[String],
    notes: &BTreeMap<usize, Vec<String>>,
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
        push_source_notes(&mut lines, indent, notes.get(&line_no).map(Vec::as_slice));
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

fn push_source_notes(lines: &mut Vec<Line<'static>>, indent: &str, notes: Option<&[String]>) {
    let Some(notes) = notes else {
        return;
    };
    for note in notes {
        lines.push(source_note_line(indent, note));
    }
}

fn source_note_line(indent: &str, note: &str) -> Line<'static> {
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
