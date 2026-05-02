use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    config::DIFF_PAGE,
    panel::markdown,
    state::{AppState, DiffSource, PendingAction},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        render_review(state, area, frame, focused);
        return;
    }

    let title = match state.diff_source {
        DiffSource::Review => "Review",
        DiffSource::Branch(_) => "Log",
        _ => "Diff",
    };
    let block = ui::framed_with_activity(
        0,
        title,
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let lines: Vec<ratatui::text::Line> = if matches!(state.diff_source, DiffSource::Branch(_)) {
        state
            .diff_text
            .split('\n')
            .map(ui::highlight_log_line)
            .collect()
    } else {
        ui::highlight_diff_text(&state.diff_text)
    };

    let max_offset = state
        .diff_line_count
        .saturating_sub(state.diff_viewport_height);
    let offset = state.diff_offset.min(max_offset);

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));

    frame.render_widget(para, area);
}

fn render_review(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let block = ui::framed_with_activity(
        0,
        "Review",
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    )
    .title_bottom(
        Line::from(Span::styled(
            "j/k move  Enter/space expand  d drill  s source  l explain  g/G top/bottom  R refresh",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
        .alignment(Alignment::Right),
    );
    let mut lines = Vec::new();
    let Some(review) = &state.review else {
        return;
    };

    for idx in visible_review_node_indices(state) {
        let node = &review.nodes[idx];
        let selected = focused && state.review_idx == idx;
        let has_children = review.nodes.iter().any(|candidate| {
            candidate
                .parent
                .as_ref()
                .is_some_and(|parent| parent == &node.id)
        });
        let has_body = renders_review_body(&node.id) && !node.body.is_empty();
        let drillable = has_drill_child(review, &node.id);
        let expanded = !state.review_collapsed.contains(&node.id);
        let marker = if has_children || has_body {
            if expanded { "▾" } else { "▸" }
        } else {
            " "
        };
        let indent = review_indent(node.depth);
        lines.push(review_title_line(
            &indent,
            marker,
            &node.title,
            node.depth,
            selected,
            drillable,
        ));

        if expanded {
            let syntax_path = review_node_path(&node.title);
            if renders_review_body(&node.id) {
                for body in &node.body {
                    let mut spans = vec![Span::styled(
                        format!("{indent}  │ "),
                        Style::default().fg(Color::DarkGray),
                    )];
                    let body_line = syntax_path
                        .map(|path| ui::highlight_diff_line_for_path(body, path))
                        .unwrap_or_else(|| ui::highlight_diff_line(body));
                    spans.extend(body_line.spans);
                    lines.push(Line::from(spans));
                }
            }
            if state.review_context_open.contains(&node.id) {
                lines.extend(review_source_context_lines(
                    review,
                    node,
                    syntax_path,
                    &indent,
                ));
            }
            if let Some(assist) = review_assist_text(state, &node.id) {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  │ ollama"),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(markdown::render(assist, &format!("{indent}  │ ")));
            }
            if has_body
                || state.review_context_open.contains(&node.id)
                || review_assist_text(state, &node.id).is_some()
            {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  └─"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    let max_offset = max_scroll_offset(state);
    let offset = state.diff_offset.min(max_offset);
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(para, area);
}

fn review_title_line(
    indent: &str,
    marker: &str,
    title: &str,
    depth: u16,
    selected: bool,
    drillable: bool,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{indent}{marker} "),
        selected_style(
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
            selected,
        ),
    )];
    spans.extend(review_title_spans(title, depth, selected));
    if drillable {
        spans.push(Span::styled(
            " ↳".to_string(),
            selected_style(
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
                selected,
            ),
        ));
    }
    Line::from(spans)
}

fn review_title_spans(title: &str, depth: u16, selected: bool) -> Vec<Span<'static>> {
    if depth == 0 {
        return vec![Span::styled(
            title.to_string(),
            selected_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                selected,
            ),
        )];
    }

    if let Some((path, rest)) = title.split_once(" in ")
        && let Some((symbol, description)) = rest.split_once(" - ")
    {
        let mut spans = styled_file_path(path, selected);
        spans.push(Span::styled(
            " in ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.push(Span::styled(
            symbol.to_string(),
            selected_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                selected,
            ),
        ));
        spans.push(Span::styled(
            " - ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.extend(styled_review_description(description, selected));
        return spans;
    }

    if let Some((location, description)) = title.split_once(" - ") {
        let mut spans = styled_file_location(location, selected);
        spans.push(Span::styled(
            " - ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.extend(styled_review_description(description, selected));
        return spans;
    }

    styled_review_description(title, selected)
}

fn styled_file_location(location: &str, selected: bool) -> Vec<Span<'static>> {
    let Some((path, line)) = location.rsplit_once(':') else {
        return styled_file_path(location, selected);
    };
    if line.chars().all(|ch| ch.is_ascii_digit()) {
        let mut spans = styled_file_path(path, selected);
        spans.push(Span::styled(
            format!(":{line}"),
            selected_style(Style::default().fg(Color::LightBlue), selected),
        ));
        spans
    } else {
        styled_file_path(location, selected)
    }
}

fn review_node_path(title: &str) -> Option<&str> {
    let location = title
        .split_once(" in ")
        .map(|(path, _)| path)
        .or_else(|| title.split_once(" - ").map(|(location, _)| location))
        .unwrap_or(title);
    let path = location
        .rsplit_once(':')
        .filter(|(_, line)| line.chars().all(|ch| ch.is_ascii_digit()))
        .map(|(path, _)| path)
        .unwrap_or(location)
        .trim();
    (!path.is_empty()).then_some(path)
}

fn is_review_file_node(node_id: &str) -> bool {
    node_id.contains(":file:")
}

fn is_review_entry_node(node_id: &str) -> bool {
    node_id.contains(":entry:")
}

fn renders_review_body(node_id: &str) -> bool {
    !is_review_file_node(node_id) && !is_review_entry_node(node_id)
}

fn styled_file_path(path: &str, selected: bool) -> Vec<Span<'static>> {
    let file_style = if is_test_review_node(path) {
        Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    };
    vec![Span::styled(
        path.to_string(),
        selected_style(file_style, selected),
    )]
}

fn styled_review_description(description: &str, selected: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = description;
    while let Some(start) = rest.find('(') {
        let (prefix, tail) = rest.split_at(start);
        if !prefix.is_empty() {
            spans.push(Span::styled(
                prefix.to_string(),
                selected_style(Style::default().fg(Color::Gray), selected),
            ));
        }
        if let Some(end) = tail.find(')') {
            let token = &tail[..=end];
            spans.extend(styled_change_token(token, selected));
            rest = &tail[end + 1..];
        } else {
            spans.push(Span::styled(
                tail.to_string(),
                selected_style(Style::default().fg(Color::Gray), selected),
            ));
            return spans;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(
            rest.to_string(),
            selected_style(Style::default().fg(Color::Gray), selected),
        ));
    }
    spans
}

fn styled_change_token(token: &str, selected: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for part in token.split_inclusive(' ') {
        let trimmed = part.trim_matches(|ch| ch == '(' || ch == ')' || ch == ' ');
        let style = if trimmed.starts_with('+') {
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else if trimmed.starts_with('-') {
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(
            part.to_string(),
            selected_style(style, selected),
        ));
    }
    spans
}

fn selected_style(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn review_source_context_lines(
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
                full_source_with_inline_diff(&section.path, &section.body, indent, multiple)
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

struct SourceSection {
    path: String,
    body: Vec<String>,
    context: Vec<String>,
}

fn source_sections(
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
        let Some(path) = review_node_path(&candidate.title) else {
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
    path: &str,
    body: &[String],
    indent: &str,
    show_path: bool,
) -> Option<Vec<Line<'static>>> {
    let text = std::fs::read_to_string(path).ok()?;
    let overlay = inline_diff_overlay(body);
    let label = if show_path {
        format!("source {path}")
    } else {
        "source".to_string()
    };
    let mut lines = vec![section_header(indent, &label)];

    for (idx, source) in text.lines().enumerate() {
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

    let eof_line = text.lines().count() + 1;
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
struct InlineDiffOverlay {
    removed_before: BTreeMap<usize, Vec<RemovedSourceLine>>,
    added_lines: BTreeSet<usize>,
}

struct RemovedSourceLine {
    old_line: Option<usize>,
    text: String,
}

fn inline_diff_overlay(body: &[String]) -> InlineDiffOverlay {
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

fn owned_spans(line: Line<'_>) -> Vec<Span<'static>> {
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect()
}

fn review_indent(depth: u16) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{}└─", "  │ ".repeat(depth.saturating_sub(1) as usize))
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        return handle_review_key(state, key);
    }

    let max_offset = state
        .diff_line_count
        .saturating_sub(state.diff_viewport_height);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.diff_offset = state.diff_offset.saturating_add(1).min(max_offset);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.diff_offset = state.diff_offset.saturating_sub(1);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.diff_offset = state.diff_offset.saturating_add(DIFF_PAGE).min(max_offset);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.diff_offset = state.diff_offset.saturating_sub(DIFF_PAGE);
        }
        KeyCode::Char('g') => {
            state.diff_offset = 0;
        }
        KeyCode::Char('G') => {
            state.diff_offset = max_offset;
        }
        KeyCode::Char('o') => {
            if let Some(path) = selected_diff_open_path(state) {
                state.pending_action = Some(PendingAction::OpenFile(path));
            } else {
                state.set_status("no source file selected", false);
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn max_scroll_offset(state: &AppState) -> u16 {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        return review_render_line_count(state)
            .min(u16::MAX as usize)
            .saturating_sub(state.diff_viewport_height as usize) as u16;
    }
    state
        .diff_line_count
        .saturating_sub(state.diff_viewport_height)
}

fn handle_review_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let visible = visible_review_node_indices(state);
    if visible.is_empty() {
        state.diff_offset = 0;
        return Ok(());
    }
    let current_pos = visible
        .iter()
        .position(|idx| *idx == state.review_idx)
        .unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(next) = visible.get(current_pos + 1) {
                state.review_idx = *next;
                ensure_review_selection_visible(state);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if current_pos > 0 {
                state.review_idx = visible[current_pos - 1];
                ensure_review_selection_visible(state);
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if let Some(review) = &state.review
                && let Some(node) = review.nodes.get(state.review_idx)
            {
                if state.review_collapsed.contains(&node.id) {
                    state.review_collapsed.remove(&node.id);
                } else {
                    state.review_collapsed.insert(node.id.clone());
                }
                clamp_review_selection(state);
                ensure_review_selection_visible(state);
            }
        }
        KeyCode::Char('d') => {
            if let Some(review) = &state.review
                && let Some(node) = review.nodes.get(state.review_idx)
            {
                state.review_collapsed.remove(&node.id);
                if let Some((child_idx, _)) = first_drill_child(review, &node.id) {
                    state.review_idx = child_idx;
                }
                ensure_review_selection_visible(state);
            }
        }
        KeyCode::Char('s') => {
            if let Some(review) = &state.review
                && let Some(node) = review.nodes.get(state.review_idx)
                && review_source_available(node)
            {
                if state.review_context_open.contains(&node.id) {
                    state.review_context_open.remove(&node.id);
                } else {
                    state.review_collapsed.remove(&node.id);
                    state.review_context_open.insert(node.id.clone());
                }
            }
        }
        KeyCode::Char('l') => {
            if let Some(review) = &state.review
                && let Some(node) = review.nodes.get(state.review_idx)
            {
                state.review_collapsed.remove(&node.id);
                state.pending_action = Some(PendingAction::ReviewAssist(node.id.clone()));
            }
        }
        KeyCode::Char('o') => {
            if let Some(path) = selected_review_open_path(state) {
                state.pending_action = Some(PendingAction::OpenFile(path));
            } else {
                state.set_status("no source file selected", false);
            }
        }
        KeyCode::Char('g') => {
            if let Some(first) = visible.first() {
                state.review_idx = *first;
            }
            state.diff_offset = 0;
        }
        KeyCode::Char('G') => {
            if let Some(last) = visible.last() {
                state.review_idx = *last;
            }
            state.diff_offset = max_scroll_offset(state);
        }
        _ => {}
    }
    Ok(())
}

fn selected_diff_open_path(state: &AppState) -> Option<String> {
    match &state.diff_source {
        DiffSource::File(path) => Some(path.clone()),
        DiffSource::Review => selected_review_open_path(state),
        DiffSource::All | DiffSource::Folder(_) | DiffSource::Commit(_) => {
            diff_path_at_offset(&state.diff_text, state.diff_offset)
        }
        DiffSource::None | DiffSource::Branch(_) => None,
    }
}

fn selected_review_open_path(state: &AppState) -> Option<String> {
    let review = state.review.as_ref()?;
    let node = review.nodes.get(state.review_idx)?;
    path_from_review_title(&node.title).or_else(|| {
        node.body
            .iter()
            .chain(node.context.iter())
            .find_map(|line| diff_path_from_line(line))
    })
}

fn path_from_review_title(title: &str) -> Option<String> {
    let path = title
        .split(" in ")
        .next()
        .unwrap_or(title)
        .split(':')
        .next()
        .unwrap_or(title)
        .trim();
    is_supported_source_path(path).then(|| path.to_string())
}

fn diff_path_at_offset(diff_text: &str, offset: u16) -> Option<String> {
    let mut current = None;
    for line in diff_text.lines().take(offset as usize + 1) {
        if let Some(path) = diff_path_from_line(line) {
            current = Some(path);
        }
    }
    current.or_else(|| diff_text.lines().find_map(diff_path_from_line))
}

fn diff_path_from_line(line: &str) -> Option<String> {
    let path = line
        .strip_prefix("diff --git a/")
        .and_then(|rest| rest.split_once(" b/").map(|(_, path)| path))
        .or_else(|| line.strip_prefix("+++ b/"))
        .or_else(|| line.strip_prefix("--- a/"))?
        .trim();
    (path != "/dev/null" && is_supported_source_path(path)).then(|| path.to_string())
}

fn is_supported_source_path(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("kt" | "kts" | "java" | "rs")
    )
}

fn first_drill_child<'a>(
    review: &'a crate::git::AssistedReview,
    node_id: &str,
) -> Option<(usize, &'a crate::git::ReviewNode)> {
    review
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.parent.as_deref() == Some(node_id))
        .find(|(_, candidate)| {
            is_review_file_node(&candidate.id) || is_review_entry_node(&candidate.id)
        })
        .or_else(|| {
            review
                .nodes
                .iter()
                .enumerate()
                .find(|(_, candidate)| candidate.parent.as_deref() == Some(node_id))
        })
}

fn has_drill_child(review: &crate::git::AssistedReview, node_id: &str) -> bool {
    review.nodes.iter().any(|candidate| {
        candidate.parent.as_deref() == Some(node_id)
            && (is_review_file_node(&candidate.id) || is_review_entry_node(&candidate.id))
    })
}

fn review_assist_text<'a>(state: &'a AppState, node_id: &str) -> Option<&'a str> {
    if let Some(job) = &state.review_assist_job
        && job.node_id == node_id
    {
        if job.output.trim().is_empty() {
            return Some("thinking...");
        }
        return Some(job.output.trim());
    }
    state.review_assists.get(node_id).map(|text| text.trim())
}

fn is_test_review_node(title: &str) -> bool {
    title.starts_with("tests/")
        || title.contains("/tests/")
        || title.contains(" in tests/")
        || title.contains(" in src/test/")
        || title.starts_with("src/test/")
        || title.contains("/src/test/")
}

fn visible_review_node_indices(state: &AppState) -> Vec<usize> {
    let Some(review) = &state.review else {
        return Vec::new();
    };
    let mut visible = Vec::new();
    for (idx, node) in review.nodes.iter().enumerate() {
        if ancestors_expanded(state, &node.id) {
            visible.push(idx);
        }
    }
    visible
}

fn review_render_line_count(state: &AppState) -> usize {
    visible_review_node_indices(state)
        .into_iter()
        .map(|idx| review_node_line_count(state, idx))
        .sum()
}

fn review_selected_line(state: &AppState) -> Option<usize> {
    let mut line = 0usize;
    for idx in visible_review_node_indices(state) {
        if idx == state.review_idx {
            return Some(line);
        }
        line += review_node_line_count(state, idx);
    }
    None
}

fn review_node_line_count(state: &AppState, idx: usize) -> usize {
    let Some(review) = &state.review else {
        return 0;
    };
    let Some(node) = review.nodes.get(idx) else {
        return 0;
    };
    let expanded = !state.review_collapsed.contains(&node.id);
    let mut count = 1usize;
    if !expanded {
        return count;
    }

    let has_body = renders_review_body(&node.id) && !node.body.is_empty();
    let context_open = state.review_context_open.contains(&node.id);
    let assist = review_assist_text(state, &node.id);
    if renders_review_body(&node.id) {
        count += node.body.len();
    }
    if context_open {
        count += review_source_context_line_count(state, node);
    }
    if let Some(text) = assist {
        count += 1 + text.lines().count();
    }
    if has_body || context_open || assist.is_some() {
        count += 1;
    }
    count
}

fn review_source_context_line_count(state: &AppState, node: &crate::git::ReviewNode) -> usize {
    if let Some(review) = &state.review {
        let sections = source_sections(review, node);
        if !sections.is_empty() {
            return 1 + sections
                .iter()
                .map(|section| {
                    if let Ok(text) = std::fs::read_to_string(&section.path) {
                        let removed_count = inline_diff_overlay(&section.body)
                            .removed_before
                            .values()
                            .map(Vec::len)
                            .sum::<usize>();
                        1 + text.lines().count() + removed_count
                    } else {
                        usize::from(!section.body.is_empty()) * (1 + section.body.len())
                            + usize::from(!section.context.is_empty()) * (1 + section.context.len())
                    }
                })
                .sum::<usize>();
        }
    }

    1 + usize::from(!node.body.is_empty()) * (1 + node.body.len())
        + usize::from(!node.context.is_empty()) * (1 + node.context.len())
}

fn review_source_available(node: &crate::git::ReviewNode) -> bool {
    if !node.context.is_empty() {
        return true;
    }
    let Some(path) = review_node_path(&node.title) else {
        return false;
    };
    !node.body.is_empty() && std::fs::read_to_string(path).is_ok()
}

fn ensure_review_selection_visible(state: &mut AppState) {
    let Some(line) = review_selected_line(state) else {
        state.diff_offset = 0;
        return;
    };
    let viewport = state.diff_viewport_height.max(1) as usize;
    let max_offset = max_scroll_offset(state) as usize;
    let mut offset = (state.diff_offset as usize).min(max_offset);
    if line < offset {
        offset = line;
    } else if line >= offset.saturating_add(viewport) {
        offset = line.saturating_add(1).saturating_sub(viewport);
    }
    state.diff_offset = offset.min(max_offset).min(u16::MAX as usize) as u16;
}

fn ancestors_expanded(state: &AppState, node_id: &str) -> bool {
    let Some(review) = &state.review else {
        return false;
    };
    let mut parent = review
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .and_then(|node| node.parent.as_deref());
    while let Some(parent_id) = parent {
        if state.review_collapsed.contains(parent_id) {
            return false;
        }
        parent = review
            .nodes
            .iter()
            .find(|node| node.id == parent_id)
            .and_then(|node| node.parent.as_deref());
    }
    true
}

fn clamp_review_selection(state: &mut AppState) {
    let visible = visible_review_node_indices(state);
    if visible.contains(&state.review_idx) {
        state.diff_offset = state.diff_offset.min(max_scroll_offset(state));
        return;
    }
    if let Some(first) = visible.first() {
        state.review_idx = *first;
    }
    state.diff_offset = state.diff_offset.min(max_scroll_offset(state));
}
