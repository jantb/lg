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
    state::{AppState, DiffViewMode, PendingAction, ReviewStyleSeverity},
    ui,
};

use super::source::{
    RenderCache, inline_diff_overlay, owned_spans, review_source_context_lines, source_sections,
};

const SUSPICIOUS_REVIEW_BG: Color = Color::Rgb(78, 57, 18);
const OK_REVIEW_STYLE_BG: Color = Color::Rgb(24, 54, 34);
const FAIL_REVIEW_STYLE_BG: Color = Color::Rgb(70, 24, 28);
const ACTIVE_REVIEW_STYLE_PULSE: [Color; 6] = [
    Color::Rgb(22, 46, 56),
    Color::Rgb(26, 62, 76),
    Color::Rgb(34, 82, 96),
    Color::Rgb(42, 106, 120),
    Color::Rgb(34, 82, 96),
    Color::Rgb(26, 62, 76),
];
const SOURCE_CHANGE_CONTEXT_LINES: u16 = 3;

pub(super) fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let title = if side_by_side_diff_enabled(state) {
        "Review: side-by-side"
    } else {
        "Review"
    };
    let block = ui::framed_with_activity(
        0,
        title,
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    )
    .title_bottom(
        Line::from(Span::styled(
            "j/k move  ↑/↓ source changes  Enter/s source  space expand  d drill  v view  f flag  l llm/pr  y copy  C chat  R refresh",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
        .alignment(Alignment::Right),
    );
    let Some(_) = &state.review else {
        return;
    };
    let lines = render_lines(state, focused, area.width.saturating_sub(2));

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

fn render_lines(state: &AppState, focused: bool, wrap_width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(review) = &state.review else {
        return lines;
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
    let body_style = Style::default().fg(Color::DarkGray);

    for idx in visible_review_node_indices(state) {
        let node = &review.nodes[idx];
        let selected = focused && state.review_idx == idx;
        let node_id = node.id.as_str();
        let has_children = parents_with_child.contains(node_id);
        let has_body = renders_review_body(node_id) && !node.body.is_empty();
        let drillable = parents_with_drill_child.contains(node_id);
        let expanded = !state.review_collapsed.contains(node_id);
        let context_open = state.review_context_open.contains(node_id);
        let assist = review_assist_text(state, node_id);
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

        if expanded || context_open || assist.is_some() {
            let syntax_path = review_node_syntax_path(&node.title);
            // Each section/marker line repeats this prefix — render it once.
            let body_prefix = format!("{indent}  │ ");
            if expanded && renders_review_body(node_id) {
                if let Some(path) = syntax_path {
                    if side_by_side_diff_enabled(state) {
                        lines.extend(prefixed_side_by_side_diff_lines(
                            &node.body,
                            path,
                            &body_prefix,
                            body_style,
                            wrap_width,
                        ));
                    } else {
                        for body in &node.body {
                            let mut spans = vec![Span::styled(body_prefix.clone(), body_style)];
                            let body_line = ui::highlight_diff_line_for_path(body, path);
                            spans.extend(owned_spans(body_line));
                            lines.push(Line::from(spans));
                        }
                    }
                } else {
                    lines.extend(markdown::render(
                        &node.body.join("\n"),
                        &body_prefix,
                        wrap_width,
                    ));
                }
            }
            if context_open {
                lines.extend(review_source_context_lines(
                    &mut cache,
                    state,
                    review,
                    node,
                    syntax_path,
                    &indent,
                ));
            }
            if let Some(assist) = assist {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  │ llm"),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(markdown::render(assist, &body_prefix, wrap_width));
            }
            if (expanded && has_body) || context_open || assist.is_some() {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  └─"),
                    body_style,
                )));
            }
        }
    }
    lines
}

fn side_by_side_diff_enabled(state: &AppState) -> bool {
    state.diff_view_mode == DiffViewMode::SideBySide
}

fn prefixed_side_by_side_diff_lines(
    body: &[String],
    path: &str,
    prefix: &str,
    prefix_style: Style,
    width: u16,
) -> Vec<Line<'static>> {
    let diff_width = width.saturating_sub(prefix.chars().count().min(u16::MAX as usize) as u16);
    ui::highlight_side_by_side_diff_text_for_path(&body.join("\n"), diff_width, path)
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
            spans.extend(owned_spans(line));
            Line::from(spans)
        })
        .collect()
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

fn is_review_hunk_node(node_id: &str) -> bool {
    node_id.contains(":hunk:")
}

fn renders_review_body(node_id: &str) -> bool {
    !is_review_file_node(node_id) && !is_review_entry_node(node_id) && !is_review_hunk_node(node_id)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewPathStyle {
    Normal,
    Active(usize),
    Finding(ReviewStyleSeverity),
}

fn review_path_style(path: &str, state: &AppState) -> ReviewPathStyle {
    if let Some(finding) = state.review_style_findings.get(path) {
        ReviewPathStyle::Finding(finding.severity)
    } else if state.review_flag_active_path.as_deref() == Some(path) {
        ReviewPathStyle::Active(state.animation_tick)
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
        ReviewPathStyle::Active(tick) => {
            file_style = file_style
                .bg(active_review_style_bg(tick))
                .add_modifier(Modifier::BOLD);
        }
        ReviewPathStyle::Normal => {}
    }
    let style = if path_style != ReviewPathStyle::Normal && selected {
        file_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        selected_style(file_style, selected)
    };
    let mut spans = Vec::new();
    if let ReviewPathStyle::Active(tick) = path_style {
        spans.push(Span::styled(
            format!(" {} FLAGGING ", active_review_marker(tick)),
            Style::default()
                .fg(Color::Black)
                .bg(active_review_style_bg(tick))
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(path.to_string(), style));
    spans
}

fn active_review_style_bg(tick: usize) -> Color {
    ACTIVE_REVIEW_STYLE_PULSE[(tick / 2) % ACTIVE_REVIEW_STYLE_PULSE.len()]
}

fn active_review_marker(tick: usize) -> &'static str {
    match (tick / 2) % 4 {
        0 => "◌",
        1 => "◐",
        2 => "●",
        _ => "◑",
    }
}

fn review_style_bg(severity: ReviewStyleSeverity) -> Color {
    match severity {
        ReviewStyleSeverity::Ok => OK_REVIEW_STYLE_BG,
        ReviewStyleSeverity::Warn => SUSPICIOUS_REVIEW_BG,
        ReviewStyleSeverity::Fail => FAIL_REVIEW_STYLE_BG,
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
        KeyCode::Char('j') => {
            move_to_next_review_node(state, &visible, current_pos);
        }
        KeyCode::Down => {
            if !jump_to_source_change(state, false) {
                move_to_next_review_node(state, &visible, current_pos);
            }
        }
        KeyCode::Char('k') => {
            move_to_previous_review_node(state, &visible, current_pos);
        }
        KeyCode::Up => {
            if !jump_to_source_change(state, true) {
                move_to_previous_review_node(state, &visible, current_pos);
            }
        }
        KeyCode::Enter => {
            if toggle_review_source(state) {
                ensure_review_selection_visible(state);
            } else {
                toggle_review_tree_node(state);
            }
        }
        KeyCode::Char(' ') => {
            toggle_review_tree_node(state);
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
            if toggle_review_source(state) {
                ensure_review_selection_visible(state);
            }
        }
        KeyCode::Char('v') => {
            state.diff_view_mode = match state.diff_view_mode {
                DiffViewMode::Unified => DiffViewMode::SideBySide,
                DiffViewMode::SideBySide => DiffViewMode::Unified,
            };
            state.diff_offset = state.diff_offset.min(super::max_scroll_offset(state));
            let label = match state.diff_view_mode {
                DiffViewMode::Unified => "unified diff",
                DiffViewMode::SideBySide => "side-by-side diff",
            };
            state.set_status(format!("showing {label}"), false);
        }
        KeyCode::Char('l') => {
            if let Some(review) = &state.review
                && let Some(node) = review.nodes.get(state.review_idx)
            {
                state.pending_action = Some(if node.id == crate::git::REVIEW_PR_TEXT_NODE_ID {
                    PendingAction::ReviewPrText
                } else {
                    PendingAction::ReviewAssist(node.id.clone())
                });
            }
        }
        KeyCode::Char('f') => {
            state.pending_action = Some(PendingAction::ReviewStyleFlags);
        }
        KeyCode::Char('y') => {
            if let Some((label, text)) = selected_review_copy_text(state) {
                state.pending_action = Some(PendingAction::CopyToClipboard { label, text });
            } else {
                state.set_status("nothing copyable for selected review item", false);
            }
        }
        KeyCode::Char('n') => {
            jump_to_review_note(state, false);
        }
        KeyCode::Char('N') => {
            jump_to_review_note(state, true);
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

fn toggle_review_tree_node(state: &mut AppState) {
    if let Some(review) = &state.review
        && let Some(node) = review.nodes.get(state.review_idx)
    {
        let node_id = node.id.clone();
        let descendant_ids = review_descendant_ids(review, &node_id);
        let has_child = review
            .nodes
            .iter()
            .any(|candidate| candidate.parent.as_deref() == Some(node_id.as_str()));
        let has_body = renders_review_body(&node_id) && !node.body.is_empty();
        if !has_child && !has_body {
            return;
        }
        if state.review_collapsed.contains(&node_id) {
            state.review_collapsed.remove(&node_id);
        } else {
            state.review_collapsed.insert(node_id.clone());
            state.review_context_open.remove(&node_id);
            state.review_context_restore_collapsed.remove(&node_id);
            for descendant_id in descendant_ids {
                state.review_collapsed.insert(descendant_id.clone());
                state.review_context_open.remove(&descendant_id);
                state
                    .review_context_restore_collapsed
                    .remove(&descendant_id);
            }
        }
        clamp_review_selection(state);
        ensure_review_selection_visible(state);
    }
}

fn review_descendant_ids(review: &crate::git::AssistedReview, node_id: &str) -> Vec<String> {
    let mut descendant_ids = Vec::new();
    collect_review_descendant_ids(review, node_id, &mut descendant_ids);
    descendant_ids
}

fn collect_review_descendant_ids(
    review: &crate::git::AssistedReview,
    node_id: &str,
    descendant_ids: &mut Vec<String>,
) {
    for candidate in &review.nodes {
        if candidate.parent.as_deref() == Some(node_id) {
            descendant_ids.push(candidate.id.clone());
            collect_review_descendant_ids(review, &candidate.id, descendant_ids);
        }
    }
}

fn toggle_review_source(state: &mut AppState) -> bool {
    let Some(review) = &state.review else {
        return false;
    };
    let Some(node) = review.nodes.get(state.review_idx) else {
        return false;
    };
    if !review_source_available(state, review, node) {
        return false;
    }

    let node_id = node.id.clone();
    if state.review_context_open.contains(&node_id) {
        state.review_context_open.remove(&node_id);
        state.review_context_restore_collapsed.remove(&node_id);
    } else {
        state.review_context_restore_collapsed.remove(&node_id);
        state.review_context_open.insert(node_id);
    }
    true
}

fn move_to_next_review_node(state: &mut AppState, visible: &[usize], current_pos: usize) {
    if let Some(next) = visible.get(current_pos + 1) {
        state.review_idx = *next;
        ensure_review_selection_visible(state);
    }
}

fn move_to_previous_review_node(state: &mut AppState, visible: &[usize], current_pos: usize) {
    if current_pos > 0 {
        state.review_idx = visible[current_pos - 1];
        ensure_review_selection_visible(state);
    }
}

fn jump_to_source_change(state: &mut AppState, previous: bool) -> bool {
    let Some(review) = &state.review else {
        return false;
    };
    let Some(node) = review.nodes.get(state.review_idx) else {
        return false;
    };
    if !state.review_context_open.contains(&node.id) {
        return false;
    }

    let lines = render_lines(state, false, state.diff_viewport_width.saturating_sub(2));
    let change_lines = source_change_group_lines(&lines);
    if change_lines.is_empty() {
        state.set_status("no source changes", false);
        return false;
    }

    let current = state
        .diff_offset
        .saturating_add(SOURCE_CHANGE_CONTEXT_LINES);
    let target = if previous {
        change_lines
            .iter()
            .rev()
            .copied()
            .find(|line| *line < current)
    } else {
        change_lines.iter().copied().find(|line| *line > current)
    };
    let Some(target) = target else {
        return false;
    };
    state.diff_offset = target
        .saturating_sub(SOURCE_CHANGE_CONTEXT_LINES)
        .min(super::max_scroll_offset(state));
    true
}

fn line_contains_source_change(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .any(|span| matches!(span.content.as_ref(), "+" | "-" | "+ " | "- "))
}

fn source_change_group_lines(lines: &[Line<'_>]) -> Vec<u16> {
    let mut groups = Vec::new();
    let mut previous_change: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        if !line_contains_source_change(line) {
            continue;
        }
        let starts_group = match previous_change {
            Some(previous) => idx > previous.saturating_add(1),
            None => true,
        };
        if starts_group {
            groups.push(idx.min(u16::MAX as usize) as u16);
        }
        previous_change = Some(idx);
    }
    groups
}

fn jump_to_review_note(state: &mut AppState, previous: bool) {
    let lines = render_lines(state, false, state.diff_viewport_width.saturating_sub(2));
    let note_lines: Vec<u16> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            line_contains_review_note(line).then_some(idx.min(u16::MAX as usize) as u16)
        })
        .collect();
    if note_lines.is_empty() {
        state.set_status("no inline review notes", false);
        return;
    }

    let current = state.diff_offset;
    let target = if previous {
        note_lines
            .iter()
            .rev()
            .copied()
            .find(|line| *line < current)
            .or_else(|| note_lines.last().copied())
    } else {
        note_lines
            .iter()
            .copied()
            .find(|line| *line > current)
            .or_else(|| note_lines.first().copied())
    }
    .unwrap_or(0);
    state.diff_offset = target.min(super::max_scroll_offset(state));
}

fn line_contains_review_note(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .any(|span| span.content.contains("review note:") || span.content.contains(" STYLE "))
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
    path_from_review_title(&node.title)
        .or_else(|| source_context_open_path(state, review, node))
        .or_else(|| {
            node.body
                .iter()
                .chain(node.context.iter())
                .find_map(|line| super::diff_path_from_line(line))
        })
}

fn source_context_open_path(
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> Option<String> {
    state
        .review_context_open
        .contains(&node.id)
        .then(|| source_sections(state, review, node).into_iter().next())?
        .map(|section| section.path)
}

fn path_from_review_title(title: &str) -> Option<String> {
    let path = review_node_path(title)?;
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
    if let Some(text) = copyable_review_assist_text(state, node_id) {
        return Some(text);
    }
    if let Some(job) = &state.review_assist_job
        && job.node_id == node_id
    {
        return Some("thinking...");
    }
    if let Some(job) = &state.review_pr_job
        && job.node_id == node_id
    {
        return Some("writing PR text...");
    }
    None
}

fn copyable_review_assist_text<'a>(state: &'a AppState, node_id: &str) -> Option<&'a str> {
    if let Some(job) = &state.review_assist_job
        && job.node_id == node_id
    {
        let output = job.output.trim();
        if !output.is_empty() {
            return Some(output);
        }
    }
    if let Some(job) = &state.review_pr_job
        && job.node_id == node_id
    {
        let output = job.output.trim();
        if !output.is_empty() {
            return Some(output);
        }
    }
    state
        .review_assists
        .get(node_id)
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
}

fn selected_review_copy_text(state: &AppState) -> Option<(String, String)> {
    let review = state.review.as_ref()?;
    let node = review.nodes.get(state.review_idx)?;
    let label = if node.id == crate::git::REVIEW_PR_TEXT_NODE_ID {
        "PR text"
    } else {
        "LLM assessment"
    };
    copyable_review_assist_text(state, &node.id).map(|text| (label.to_string(), text.to_owned()))
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
    let context_open = state.review_context_open.contains(&node.id);
    let assist = review_assist_text(state, &node.id);
    if !expanded && !context_open && assist.is_none() {
        return count;
    }

    let has_body = renders_review_body(&node.id) && !node.body.is_empty();
    if expanded && renders_review_body(&node.id) {
        count += review_body_line_count(state, node);
    }
    if context_open {
        count += review_source_context_line_count(state, node);
    }
    if let Some(text) = assist {
        count += 1 + text.lines().count();
    }
    if (expanded && has_body) || context_open || assist.is_some() {
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
    if side_by_side_diff_enabled(state) {
        ui::side_by_side_diff_line_count(&node.body.join("\n"), state.diff_viewport_width)
    } else {
        node.body.len()
    }
}

fn review_source_context_line_count(state: &AppState, node: &crate::git::ReviewNode) -> usize {
    if let Some(review) = &state.review {
        let sections = source_sections(state, review, node);
        if !sections.is_empty() {
            return 1 + sections
                .iter()
                .map(|section| {
                    let note_count = section.notes.values().map(Vec::len).sum::<usize>();
                    if let Ok(text) = std::fs::read_to_string(&section.path) {
                        let removed_count = inline_diff_overlay(&section.body)
                            .removed_before
                            .values()
                            .map(Vec::len)
                            .sum::<usize>();
                        1 + text.lines().count() + removed_count + note_count
                    } else {
                        usize::from(!section.body.is_empty()) * (1 + section.body.len())
                            + usize::from(!section.context.is_empty()) * (1 + section.context.len())
                            + usize::from(note_count > 0) * (1 + note_count)
                    }
                })
                .sum::<usize>();
        }
    }

    1 + usize::from(!node.body.is_empty()) * (1 + node.body.len())
        + usize::from(!node.context.is_empty()) * (1 + node.context.len())
}

fn review_source_available(
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> bool {
    if !source_sections(state, review, node).is_empty() {
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
