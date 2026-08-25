//! What the review keys do: expanding, jumping, and copying.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, text::Line};

use crate::state::{AppState, DiffViewMode, PendingAction};

use super::super::source::source_sections;
use super::*;

const SOURCE_CHANGE_CONTEXT_LINES: u16 = 3;

pub(in crate::panel::main) fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    let visible = visible_review_node_indices(state);
    if visible.is_empty() {
        state.diff_offset = 0;
        // An empty review has nothing to move through, but the keys are still
        // review mode's; reporting them as unbound would be wrong.
        return Ok(true);
    }
    state.diff_offset = state
        .diff_offset
        .min(crate::panel::main::max_scroll_offset(state));
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
            state.diff_offset = state
                .diff_offset
                .min(crate::panel::main::max_scroll_offset(state));
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
            state.diff_offset = crate::panel::main::max_scroll_offset(state);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(in crate::panel::main) fn toggle_review_tree_node(state: &mut AppState) {
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

pub(in crate::panel::main) fn review_descendant_ids(
    review: &crate::git::AssistedReview,
    node_id: &str,
) -> Vec<String> {
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
        .min(crate::panel::main::max_scroll_offset(state));
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
    state.diff_offset = target.min(crate::panel::main::max_scroll_offset(state));
}

fn line_contains_review_note(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .any(|span| span.content.contains("review note:") || span.content.contains(" STYLE "))
}

pub(in crate::panel::main) fn select_mouse_row(state: &mut AppState, area: Rect, row: u16) {
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

pub(in crate::panel::main) fn selected_open_path(state: &AppState) -> Option<String> {
    let review = state.review.as_ref()?;
    let node = review.nodes.get(state.review_idx)?;
    path_from_review_title(&node.title)
        .or_else(|| source_context_open_path(state, review, node))
        .or_else(|| {
            node.body
                .iter()
                .chain(node.context.iter())
                .find_map(|line| crate::panel::main::diff_path_from_line(line))
        })
}

pub(in crate::panel::main) fn source_context_open_path(
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

pub(in crate::panel::main) fn review_assist_text<'a>(
    state: &'a AppState,
    node_id: &str,
) -> Option<&'a str> {
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

pub(in crate::panel::main) fn copyable_review_assist_text<'a>(
    state: &'a AppState,
    node_id: &str,
) -> Option<&'a str> {
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

pub(in crate::panel::main) fn selected_review_copy_text(
    state: &AppState,
) -> Option<(String, String)> {
    let review = state.review.as_ref()?;
    let node = review.nodes.get(state.review_idx)?;
    let label = if node.id == crate::git::REVIEW_PR_TEXT_NODE_ID {
        "PR text"
    } else {
        "LLM assessment"
    };
    copyable_review_assist_text(state, &node.id).map(|text| (label.to_string(), text.to_owned()))
}
