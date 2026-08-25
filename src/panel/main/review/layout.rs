//! How many lines the review pane draws, and keeping the selection in view.

use crate::{panel::markdown, state::AppState, ui};

use super::super::source::{inline_diff_overlay, source_sections};
use super::*;

pub(in crate::panel::main) fn visible_review_node_indices(state: &AppState) -> Vec<usize> {
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

pub(in crate::panel::main) fn render_line_count(state: &AppState) -> usize {
    visible_review_node_indices(state)
        .into_iter()
        .map(|idx| review_node_line_count(state, idx))
        .sum()
}

pub(in crate::panel::main) fn review_selected_line(state: &AppState) -> Option<usize> {
    let mut line = 0usize;
    for idx in visible_review_node_indices(state) {
        if idx == state.review_idx {
            return Some(line);
        }
        line += review_node_line_count(state, idx);
    }
    None
}

pub(in crate::panel::main) fn review_node_line_count(state: &AppState, idx: usize) -> usize {
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

pub(in crate::panel::main) fn review_body_line_count(
    state: &AppState,
    node: &crate::git::ReviewNode,
) -> usize {
    let indent = review_indent(node.depth);
    let prefix = format!("{indent}  │ ");
    let Some(path) = review_node_syntax_path(&node.title) else {
        return markdown::render(
            &node.body.join("\n"),
            &prefix,
            state.diff_viewport_width.saturating_sub(2),
        )
        .len();
    };
    // Count the wrapped rows the body actually draws, not its source lines.
    let diff_width = diff_body_width(state.diff_viewport_width, &prefix);
    if side_by_side_diff_enabled(state) {
        ui::side_by_side_diff_line_count(&node.body.join("\n"), diff_width)
    } else {
        node.body
            .iter()
            .map(|line| ui::highlight_diff_line_wrapped_for_path(line, path, diff_width).len())
            .sum()
    }
}

pub(in crate::panel::main) fn review_source_context_line_count(
    state: &AppState,
    node: &crate::git::ReviewNode,
) -> usize {
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

pub(in crate::panel::main) fn review_source_available(
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

pub(in crate::panel::main) fn ensure_review_selection_visible(state: &mut AppState) {
    let Some(line) = review_selected_line(state) else {
        state.diff_offset = 0;
        return;
    };
    let viewport = state.diff_viewport_height.max(1) as usize;
    let max_offset = crate::panel::main::max_scroll_offset(state) as usize;
    let offset = crate::panel::scroll::selection_scroll_offset(
        Some(line),
        render_line_count(state),
        viewport,
        state.diff_offset as usize,
    );
    state.diff_offset = offset.min(max_offset).min(u16::MAX as usize) as u16;
}

pub(in crate::panel::main) fn ancestors_expanded(state: &AppState, node_id: &str) -> bool {
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

pub(in crate::panel::main) fn clamp_review_selection(state: &mut AppState) {
    let visible = visible_review_node_indices(state);
    if visible.contains(&state.review_idx) {
        state.diff_offset = state
            .diff_offset
            .min(crate::panel::main::max_scroll_offset(state));
        return;
    }
    if let Some(first) = visible.first() {
        state.review_idx = *first;
    }
    state.diff_offset = state
        .diff_offset
        .min(crate::panel::main::max_scroll_offset(state));
}
