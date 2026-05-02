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

    let title = if matches!(state.diff_source, DiffSource::Review) {
        "Review"
    } else {
        "Diff"
    };
    let block = ui::framed_with_activity(
        0,
        title,
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let lines: Vec<ratatui::text::Line> = state
        .diff_text
        .split('\n')
        .map(ui::highlight_diff_line)
        .collect();

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
        let style = if selected {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else if node.depth == 0 {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if node.title.ends_with('/') {
            Style::default().fg(Color::Blue)
        } else if is_test_review_node(&node.title) {
            Style::default().fg(Color::LightMagenta)
        } else if is_source_review_node(&node.title) {
            Style::default().fg(Color::LightGreen)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{indent}{marker} {}", node.title),
            style,
        )));

        if expanded {
            for body in &node.body {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{indent}  │ "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    ui::highlight_diff_line(body).spans.remove(0),
                ]));
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

    let max_offset = lines
        .len()
        .min(u16::MAX as usize)
        .saturating_sub(state.diff_viewport_height as usize) as u16;
    let offset = state.diff_offset.min(max_offset);
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(para, area);
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

fn handle_review_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let visible = visible_review_node_indices(state);
    let current_pos = visible
        .iter()
        .position(|idx| *idx == state.review_idx)
        .unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(next) = visible.get(current_pos + 1) {
                state.review_idx = *next;
                state.diff_offset = state.diff_offset.saturating_add(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if current_pos > 0 {
                state.review_idx = visible[current_pos - 1];
                state.diff_offset = state.diff_offset.saturating_sub(1);
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
            state.diff_offset = u16::MAX;
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

fn is_source_review_node(title: &str) -> bool {
    title.starts_with("src/")
        || title.contains(" in src/")
        || title.ends_with(".rs")
        || title.ends_with(".kt")
        || title.ends_with(".kts")
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
        return;
    }
    if let Some(first) = visible.first() {
        state.review_idx = *first;
    }
}
