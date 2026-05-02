use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    config::DIFF_PAGE,
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
            "j/k move  Enter/space expand  s source  l explain  g/G top/bottom  R refresh",
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
        let has_body = !node.body.is_empty();
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
        ));

        if expanded {
            let syntax_path = review_node_path(&node.title);
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
            if state.review_context_open.contains(&node.id) {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  │ source context"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                for context in &node.context {
                    lines.push(Line::from(Span::styled(
                        format!("{indent}  │ {context}"),
                        Style::default().fg(Color::Gray),
                    )));
                }
            }
            if let Some(assist) = review_assist_text(state, &node.id) {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  │ ollama"),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                )));
                for line in assist.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("{indent}  │ {line}"),
                        Style::default().fg(Color::Gray),
                    )));
                }
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
        KeyCode::Char('s') => {
            if let Some(review) = &state.review
                && let Some(node) = review.nodes.get(state.review_idx)
                && !node.context.is_empty()
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

    let has_body = !node.body.is_empty();
    let context_open = state.review_context_open.contains(&node.id);
    let assist = review_assist_text(state, &node.id);
    count += node.body.len();
    if context_open {
        count += 1 + node.context.len();
    }
    if let Some(text) = assist {
        count += 1 + text.lines().count();
    }
    if has_body || context_open || assist.is_some() {
        count += 1;
    }
    count
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
