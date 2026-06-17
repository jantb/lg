use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::{
    state::{AppState, DiffViewMode},
    ui,
};

const STYLE_WARN_NOTE_BG: Color = Color::Rgb(92, 68, 18);
const STYLE_FAIL_NOTE_BG: Color = Color::Rgb(88, 24, 30);
const STYLE_WARN_LABEL_BG: Color = Color::Yellow;
const STYLE_FAIL_LABEL_BG: Color = Color::Red;
const INLINE_ADDED_BG: Color = Color::Rgb(24, 54, 34);
const INLINE_REMOVED_BG: Color = Color::Rgb(60, 28, 38);
const SIDE_SEPARATOR: &str = " | ";

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
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    syntax_path: Option<&str>,
    indent: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![section_header(indent, "source context")];
    let sections = source_sections(state, review, node);
    let side_by_side = side_by_side_diff_enabled(state);
    let viewport_width = state.diff_viewport_width;
    if !sections.is_empty() {
        let multiple = sections.len() > 1;
        for section in sections {
            if let Some(mut source) = full_source_with_inline_diff(
                cache,
                &section.path,
                &section.body,
                &section.notes,
                indent,
                SourceRenderOptions {
                    show_path: multiple,
                    side_by_side,
                    viewport_width,
                },
            ) {
                lines.append(&mut source);
            } else {
                lines.extend(fallback_source_context_lines(
                    &section.body,
                    &section.context,
                    Some(&section.path),
                    &section.notes,
                    indent,
                    side_by_side,
                    viewport_width,
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
        side_by_side,
        viewport_width,
    ));
    lines
}

fn side_by_side_diff_enabled(state: &AppState) -> bool {
    state.diff_view_mode == DiffViewMode::SideBySide
}

pub(super) fn source_sections(
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> Vec<SourceSection> {
    if is_full_diff_root(node)
        && let Some(sections) = full_diff_source_sections(review)
        && !sections.is_empty()
    {
        let mut sections = sections;
        attach_inline_review_notes(state, review, node, &mut sections);
        return sections;
    }

    let mut sections = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut notes_by_path = inline_review_notes_by_path(state, review, node);
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
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    sections: &mut [SourceSection],
) {
    let mut notes_by_path = inline_review_notes_by_path(state, review, node);
    for section in sections {
        section.notes = notes_by_path.remove(&section.path).unwrap_or_default();
    }
}

fn inline_review_notes_by_path(
    state: &AppState,
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
        let Some(path) = super::review::review_node_path(&candidate.title) else {
            continue;
        };
        let Some(line) = review_node_line(&candidate.title)
            .or_else(|| first_body_hunk_new_line(&candidate.body))
            .map(|line| line.max(1))
        else {
            continue;
        };
        if let Some(note) = inline_review_note_text(candidate) {
            push_inline_note(&mut notes, path, line, note);
        }
        if let Some(assist) = inline_assist_note_text(state, &candidate.id) {
            push_inline_note(&mut notes, path, line, assist);
        }
    }

    for (path, finding) in &state.review_style_findings {
        if !source_sections_contain_path(review, node, path) {
            continue;
        }
        if matches!(finding.severity, crate::state::ReviewStyleSeverity::Ok) {
            continue;
        }
        let line = style_finding_line(review, node, path, finding).unwrap_or(1);
        push_inline_note(
            &mut notes,
            path,
            line,
            format!(
                "style {}: {}",
                finding.severity.label().to_ascii_lowercase(),
                finding.reason.trim()
            ),
        );
    }
    notes
}

fn style_finding_line(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
    finding: &crate::state::ReviewStyleFinding,
) -> Option<usize> {
    finding
        .line
        .or_else(|| line_matching_reason(review, node, path, &finding.reason))
        .or_else(|| first_changed_line_for_path(review, node, path))
        .map(|line| line.max(1))
}

fn push_inline_note(
    notes: &mut BTreeMap<String, BTreeMap<usize, Vec<String>>>,
    path: &str,
    line: usize,
    note: String,
) {
    let line_notes = notes
        .entry(path.to_string())
        .or_default()
        .entry(line.max(1))
        .or_default();
    if !line_notes.contains(&note) {
        line_notes.push(note);
    }
}

fn source_sections_contain_path(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
) -> bool {
    std::iter::once(node)
        .chain(
            review
                .nodes
                .iter()
                .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
        )
        .any(|candidate| super::review::review_node_path(&candidate.title) == Some(path))
        || (is_full_diff_root(node)
            && full_diff_source_sections(review)
                .is_some_and(|sections| sections.iter().any(|section| section.path == path)))
}

fn first_changed_line_for_path(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
) -> Option<usize> {
    std::iter::once(node)
        .chain(
            review
                .nodes
                .iter()
                .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
        )
        .filter(|candidate| super::review::review_node_path(&candidate.title) == Some(path))
        .find_map(|candidate| {
            review_node_line(&candidate.title).or_else(|| first_body_hunk_new_line(&candidate.body))
        })
        .or_else(|| {
            full_diff_source_sections(review)?
                .into_iter()
                .find(|section| section.path == path)
                .and_then(|section| first_body_hunk_new_line(&section.body))
        })
        .map(|line| line.max(1))
}

fn line_matching_reason(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
    reason: &str,
) -> Option<usize> {
    let fragments = code_fragments(reason);
    if fragments.is_empty() {
        return None;
    }
    std::iter::once(node)
        .chain(
            review
                .nodes
                .iter()
                .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
        )
        .filter(|candidate| super::review::review_node_path(&candidate.title) == Some(path))
        .find_map(|candidate| {
            line_matching_reason_in_body(&candidate.body, &fragments)
                .or_else(|| line_matching_reason_in_context(&candidate.context, &fragments))
        })
        .or_else(|| {
            full_diff_source_sections(review)?
                .into_iter()
                .find(|section| section.path == path)
                .and_then(|section| line_matching_reason_in_body(&section.body, &fragments))
        })
        .or_else(|| line_matching_reason_in_file(path, &fragments))
}

fn line_matching_reason_in_body(body: &[String], fragments: &[String]) -> Option<usize> {
    let mut new_line = 0usize;
    let mut in_hunk = false;
    for line in body {
        if let Some((_, new_start)) = parse_hunk_header(line) {
            new_line = new_start;
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.starts_with("\\ No newline") {
            continue;
        }
        if line.starts_with('-') {
            continue;
        }
        let Some(source) = line.strip_prefix('+').or_else(|| line.strip_prefix(' ')) else {
            continue;
        };
        if source_matches_fragments(source, fragments) {
            return Some(new_line.max(1));
        }
        new_line = new_line.saturating_add(1);
    }
    None
}

fn line_matching_reason_in_context(context: &[String], fragments: &[String]) -> Option<usize> {
    context.iter().find_map(|line| {
        let (line_no, source) = line.split_once(" | ")?;
        source_matches_fragments(source, fragments)
            .then(|| line_no.trim().parse().ok())
            .flatten()
    })
}

fn line_matching_reason_in_file(path: &str, fragments: &[String]) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .enumerate()
        .find_map(|(idx, line)| source_matches_fragments(line, fragments).then_some(idx + 1))
}

fn code_fragments(reason: &str) -> Vec<String> {
    reason
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ':' | '`' | '"' | '\''))
        .map(|part| {
            part.trim_matches(|ch: char| {
                matches!(ch, '.' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>')
            })
        })
        .filter(|part| {
            part.len() >= 8
                && part
                    .chars()
                    .any(|ch| matches!(ch, '.' | '(' | ')' | '_') || ch.is_ascii_uppercase())
        })
        .map(str::to_string)
        .collect()
}

fn source_matches_fragments(source: &str, fragments: &[String]) -> bool {
    let compact_source = compact_code(source);
    fragments
        .iter()
        .map(|fragment| compact_code(fragment))
        .any(|fragment| !fragment.is_empty() && compact_source.contains(&fragment))
}

fn compact_code(s: &str) -> String {
    s.chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>()
}

fn inline_assist_note_text(state: &AppState, node_id: &str) -> Option<String> {
    let text = if let Some(job) = &state.review_assist_job
        && job.node_id == node_id
    {
        if job.output.trim().is_empty() {
            "thinking..."
        } else {
            job.output.trim()
        }
    } else {
        state.review_assists.get(node_id)?.trim()
    };
    first_note_line(text).map(|line| format!("llm: {line}"))
}

fn first_note_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| {
            line.trim()
                .trim_start_matches('#')
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim()
        })
        .find(|line| !line.is_empty())
        .map(|line| truncate_note(line, 120))
}

fn truncate_note(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in line.chars().take(max_chars) {
        out.push(ch);
    }
    if line.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn inline_review_note_text(node: &crate::git::ReviewNode) -> Option<String> {
    if let Some(effect) = node
        .body
        .iter()
        .find_map(|line| line.trim().strip_prefix("effect: "))
    {
        let effect = effect.trim();
        if !effect.is_empty() {
            return Some(format!("entry point: {effect}"));
        }
    }

    if (node.id.contains(":entry:") || node.id.contains(":hunk:"))
        && let Some((_, description)) = node.title.split_once(" - ")
    {
        let description = description.trim();
        if !description.is_empty() {
            return Some(format!("entry point: {description}"));
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
    side_by_side: bool,
    viewport_width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !body.is_empty() {
        lines.push(section_header(indent, "diff"));
        if side_by_side {
            lines.extend(prefixed_side_by_side_diff_lines(
                body,
                syntax_path,
                indent,
                viewport_width,
            ));
        } else {
            for body in body {
                let mut spans = context_prefix(indent);
                let body_line = syntax_path
                    .map(|path| ui::highlight_diff_line_for_path(body, path))
                    .unwrap_or_else(|| ui::highlight_diff_line(body));
                spans.extend(owned_spans(body_line));
                lines.push(Line::from(spans));
            }
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

struct SourceRenderOptions {
    show_path: bool,
    side_by_side: bool,
    viewport_width: u16,
}

fn full_source_with_inline_diff(
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
pub(super) struct InlineDiffOverlay {
    pub removed_before: BTreeMap<usize, Vec<RemovedSourceLine>>,
    pub added_lines: BTreeSet<usize>,
    pub old_line_for_new: BTreeMap<usize, usize>,
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

fn entry_point_note_line(indent: &str, reason: &str) -> Line<'static> {
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

fn style_note_parts(note: &str) -> Option<(crate::state::ReviewStyleSeverity, &str)> {
    note.strip_prefix("style warn: ")
        .map(|reason| (crate::state::ReviewStyleSeverity::Warn, reason))
        .or_else(|| {
            note.strip_prefix("style fail: ")
                .map(|reason| (crate::state::ReviewStyleSeverity::Fail, reason))
        })
}

fn style_note_line(
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

fn context_prefix_width(indent: &str) -> usize {
    indent.chars().count() + "  │ ".chars().count()
}

fn prefixed_side_by_side_diff_lines(
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
