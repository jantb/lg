use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::collections::HashSet;

use crate::{
    panel::markdown,
    state::{AppState, PendingAction, ReviewStyleSeverity},
    ui,
};

use super::source::{
    RenderCache, inline_diff_overlay, review_source_context_lines, source_sections,
};

const SUSPICIOUS_REVIEW_BG: Color = Color::Rgb(78, 57, 18);
const OK_REVIEW_STYLE_BG: Color = Color::Rgb(24, 54, 34);
const FAIL_REVIEW_STYLE_BG: Color = Color::Rgb(70, 24, 28);
const ACTIVE_REVIEW_STYLE_BG: Color = Color::Rgb(28, 48, 70);

pub(super) fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
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
            "j/k move  Enter/space expand  d drill  s source  l explain  C chat  g/G top/bottom  R refresh",
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

    // Precompute parent membership sets so the inner loop is O(1) per node
    // instead of repeatedly scanning every node. Built once per frame.
    let mut parents_with_child: HashSet<&str> = HashSet::new();
    let mut parents_with_drill_child: HashSet<&str> = HashSet::new();
    for candidate in &review.nodes {
        if let Some(parent) = candidate.parent.as_deref() {
            parents_with_child.insert(parent);
            if is_review_file_node(&candidate.id) || is_review_entry_node(&candidate.id) {
                parents_with_drill_child.insert(parent);
            }
        }
    }

    // File-read cache shared across this frame's source-context renders.
    let mut cache = RenderCache::default();
    let wrap_width = area.width.saturating_sub(2);
    let body_style = Style::default().fg(Color::DarkGray);

    for idx in visible_review_node_indices(state) {
        let node = &review.nodes[idx];
        let selected = focused && state.review_idx == idx;
        let node_id = node.id.as_str();
        let has_children = parents_with_child.contains(node_id);
        let has_body = renders_review_body(node_id) && !node.body.is_empty();
        let drillable = parents_with_drill_child.contains(node_id);
        let expanded = !state.review_collapsed.contains(node_id);
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
            state,
        ));

        if expanded {
            let syntax_path = review_node_syntax_path(&node.title);
            // Each section/marker line repeats this prefix — render it once.
            let body_prefix = format!("{indent}  │ ");
            if renders_review_body(node_id) {
                if let Some(path) = syntax_path {
                    for body in &node.body {
                        let mut spans = vec![Span::styled(body_prefix.clone(), body_style)];
                        let body_line = ui::highlight_diff_line_for_path(body, path);
                        spans.extend(body_line.spans);
                        lines.push(Line::from(spans));
                    }
                } else {
                    lines.extend(markdown::render(
                        &node.body.join("\n"),
                        &body_prefix,
                        wrap_width,
                    ));
                }
            }
            if state.review_context_open.contains(node_id) {
                lines.extend(review_source_context_lines(
                    &mut cache,
                    review,
                    node,
                    syntax_path,
                    &indent,
                ));
            }
            let assist = review_assist_text(state, node_id);
            let style_finding = review_style_finding_text(state, &node.title);
            if let Some((severity, reason)) = style_finding {
                lines.push(Line::from(vec![
                    Span::styled(body_prefix.clone(), body_style),
                    Span::styled(
                        format!("style {}: ", severity.label().to_ascii_lowercase()),
                        Style::default()
                            .fg(review_style_fg(severity))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(reason.to_string(), Style::default().fg(Color::Gray)),
                ]));
            }
            if let Some(assist) = assist {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  │ ollama"),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(markdown::render(assist, &body_prefix, wrap_width));
            }
            if has_body
                || state.review_context_open.contains(node_id)
                || assist.is_some()
                || style_finding.is_some()
            {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  └─"),
                    body_style,
                )));
            }
        }
    }

    // Use the lines we just built as the source of truth for the scroll bound.
    // Avoids a second walk over every visible node (each of which would
    // re-read source files and re-parse diff overlays) and stays correct when
    // markdown::render word-wraps assist output into more lines than the raw
    // source had.
    let max_offset = super::scroll_bound(lines.len(), area.height.saturating_sub(2));
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
    state: &AppState,
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
    spans.extend(review_title_spans(title, depth, selected, state));
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

fn review_title_spans(
    title: &str,
    depth: u16,
    selected: bool,
    state: &AppState,
) -> Vec<Span<'static>> {
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
        let mut spans = styled_file_path(path, selected, review_path_style(path, state));
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
        let mut spans = styled_file_location(location, selected, state);
        spans.push(Span::styled(
            " - ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.extend(styled_review_description(description, selected));
        return spans;
    }

    styled_review_description(title, selected)
}

fn styled_file_location(location: &str, selected: bool, state: &AppState) -> Vec<Span<'static>> {
    let Some((path, line)) = location.rsplit_once(':') else {
        return styled_file_path(location, selected, review_path_style(location, state));
    };
    if line.chars().all(|ch| ch.is_ascii_digit()) {
        let mut spans = styled_file_path(path, selected, review_path_style(path, state));
        spans.push(Span::styled(
            format!(":{line}"),
            selected_style(Style::default().fg(Color::LightBlue), selected),
        ));
        spans
    } else {
        styled_file_path(location, selected, review_path_style(location, state))
    }
}

pub(super) fn review_node_path(title: &str) -> Option<&str> {
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

fn review_node_syntax_path(title: &str) -> Option<&str> {
    review_node_path(title).filter(|path| super::is_supported_source_path(path))
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewPathStyle {
    Normal,
    Active,
    Finding(ReviewStyleSeverity),
}

fn review_path_style(path: &str, state: &AppState) -> ReviewPathStyle {
    if let Some(finding) = state.review_style_findings.get(path) {
        ReviewPathStyle::Finding(finding.severity)
    } else if state.review_flag_active_path.as_deref() == Some(path) {
        ReviewPathStyle::Active
    } else {
        ReviewPathStyle::Normal
    }
}

fn styled_file_path(path: &str, selected: bool, path_style: ReviewPathStyle) -> Vec<Span<'static>> {
    let mut file_style = if is_test_review_node(path) {
        Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    };
    match path_style {
        ReviewPathStyle::Finding(severity) => {
            file_style = file_style.bg(review_style_bg(severity));
        }
        ReviewPathStyle::Active => {
            file_style = file_style.bg(ACTIVE_REVIEW_STYLE_BG);
        }
        ReviewPathStyle::Normal => {}
    }
    let style = if path_style != ReviewPathStyle::Normal && selected {
        file_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        selected_style(file_style, selected)
    };
    vec![Span::styled(path.to_string(), style)]
}

fn review_style_finding_text<'a>(
    state: &'a AppState,
    title: &str,
) -> Option<(ReviewStyleSeverity, &'a str)> {
    let path = review_node_path(title)?;
    let finding = state.review_style_findings.get(path)?;
    Some((finding.severity, finding.reason.trim()))
}

fn review_style_bg(severity: ReviewStyleSeverity) -> Color {
    match severity {
        ReviewStyleSeverity::Ok => OK_REVIEW_STYLE_BG,
        ReviewStyleSeverity::Warn => SUSPICIOUS_REVIEW_BG,
        ReviewStyleSeverity::Fail => FAIL_REVIEW_STYLE_BG,
    }
}

fn review_style_fg(severity: ReviewStyleSeverity) -> Color {
    match severity {
        ReviewStyleSeverity::Ok => Color::LightGreen,
        ReviewStyleSeverity::Warn => Color::LightYellow,
        ReviewStyleSeverity::Fail => Color::LightRed,
    }
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

fn review_indent(depth: u16) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{}└─", "  │ ".repeat(depth.saturating_sub(1) as usize))
    }
}

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let visible = visible_review_node_indices(state);
    if visible.is_empty() {
        state.diff_offset = 0;
        return Ok(());
    }
    state.diff_offset = state.diff_offset.min(super::max_scroll_offset(state));
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
                && review_source_available(review, node)
            {
                if state.review_context_open.contains(&node.id) {
                    state.review_context_open.remove(&node.id);
                    if state.review_context_restore_collapsed.remove(&node.id) {
                        state.review_collapsed.insert(node.id.clone());
                        clamp_review_selection(state);
                    }
                } else {
                    if state.review_collapsed.contains(&node.id) {
                        state
                            .review_context_restore_collapsed
                            .insert(node.id.clone());
                    } else {
                        state.review_context_restore_collapsed.remove(&node.id);
                    }
                    state.review_collapsed.remove(&node.id);
                    state.review_context_open.insert(node.id.clone());
                }
                ensure_review_selection_visible(state);
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
        KeyCode::Char('C') => {
            state.modal = crate::state::Modal::ReviewChat;
            state.review_chat_cursor = state.review_chat_input.chars().count();
        }
        KeyCode::Char('o') => {
            if let Some(path) = selected_open_path(state) {
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
            state.diff_offset = super::max_scroll_offset(state);
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn select_mouse_row(state: &mut AppState, area: Rect, row: u16) {
    if row <= area.y || row >= area.y.saturating_add(area.height).saturating_sub(1) {
        return;
    }
    let visual_line = row
        .saturating_sub(area.y)
        .saturating_sub(1)
        .saturating_add(state.diff_offset) as usize;
    let mut line = 0usize;
    for idx in visible_review_node_indices(state) {
        let count = review_node_line_count(state, idx);
        if visual_line < line.saturating_add(count) {
            state.review_idx = idx;
            return;
        }
        line = line.saturating_add(count);
    }
}

pub(super) fn selected_open_path(state: &AppState) -> Option<String> {
    let review = state.review.as_ref()?;
    let node = review.nodes.get(state.review_idx)?;
    path_from_review_title(&node.title).or_else(|| {
        node.body
            .iter()
            .chain(node.context.iter())
            .find_map(|line| super::diff_path_from_line(line))
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
    super::is_supported_source_path(path).then(|| path.to_string())
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

pub(super) fn render_line_count(state: &AppState) -> usize {
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
        count += review_body_line_count(state, node);
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

fn review_body_line_count(state: &AppState, node: &crate::git::ReviewNode) -> usize {
    let Some(_path) = review_node_syntax_path(&node.title) else {
        let indent = review_indent(node.depth);
        let prefix = format!("{indent}  │ ");
        return markdown::render(
            &node.body.join("\n"),
            &prefix,
            state.diff_viewport_width.saturating_sub(2),
        )
        .len();
    };
    node.body.len()
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

fn review_source_available(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> bool {
    if !source_sections(review, node).is_empty() {
        return true;
    }
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
    let max_offset = super::max_scroll_offset(state) as usize;
    let offset = crate::panel::scroll::selection_scroll_offset(
        Some(line),
        render_line_count(state),
        viewport,
        state.diff_offset as usize,
    );
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
        state.diff_offset = state.diff_offset.min(super::max_scroll_offset(state));
        return;
    }
    if let Some(first) = visible.first() {
        state.review_idx = *first;
    }
    state.diff_offset = state.diff_offset.min(super::max_scroll_offset(state));
}
