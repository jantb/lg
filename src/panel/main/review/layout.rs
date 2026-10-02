//! How many lines the review pane draws, and keeping the selection in view.

use crate::state::AppState;

use super::super::source::source_sections;
use super::*;

pub(in crate::panel::main) fn visible_review_node_indices(state: &AppState) -> Vec<usize> {
    let Some(review) = &state.review.assisted else {
        return Vec::new();
    };
    // One pass, parents before children: a node shows when its parent shows
    // and is expanded. Walking every node's ancestors instead made this
    // quadratic, and it runs several times a frame.
    let mut open: std::collections::HashMap<&str, bool> =
        std::collections::HashMap::with_capacity(review.nodes.len());
    let mut visible = Vec::new();
    for (idx, node) in review.nodes.iter().enumerate() {
        let shown = match node.parent.as_deref() {
            None => true,
            Some(parent) => match open.get(parent) {
                Some(&parent_open) => parent_open,
                None => ancestors_expanded(state, &node.id),
            },
        };
        open.entry(node.id.as_str())
            .or_insert(shown && !state.review.collapsed.contains(&node.id));
        if shown {
            visible.push(idx);
        }
    }
    visible
}

pub(in crate::panel::main) fn render_line_count(state: &AppState) -> usize {
    let pass = review_pass(state);
    visible_review_node_indices(state)
        .into_iter()
        .map(|idx| review_node_line_count(state, pass, idx))
        .sum()
}

pub(in crate::panel::main) fn review_selected_line(state: &AppState) -> Option<usize> {
    let pass = review_pass(state);
    let mut line = 0usize;
    for idx in visible_review_node_indices(state) {
        if idx == state.review.idx {
            return Some(line);
        }
        line += review_node_line_count(state, pass, idx);
    }
    None
}

/// The lines node `idx` takes on screen, its title included: counted off the
/// lines the pane draws for it, so the scroll bound, the mouse and the jumps
/// all agree with what is drawn.
pub(in crate::panel::main) fn review_node_line_count(
    state: &AppState,
    pass: ReviewPass,
    idx: usize,
) -> usize {
    if state
        .review
        .assisted
        .as_ref()
        .is_none_or(|review| idx >= review.nodes.len())
    {
        return 0;
    }
    node_line_count(state, idx, pass)
}

/// A pass over the nodes at the width the pane wraps to.
pub(in crate::panel::main) fn review_pass(state: &AppState) -> ReviewPass {
    ReviewPass::new(state, review_wrap_width(state))
}

/// The width the review pane wraps its lines to: the inside of the main pane.
pub(in crate::panel::main) fn review_wrap_width(state: &AppState) -> u16 {
    state.diff_viewport_width
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
    let Some(review) = &state.review.assisted else {
        return false;
    };
    let mut parent = review
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .and_then(|node| node.parent.as_deref());
    while let Some(parent_id) = parent {
        if state.review.collapsed.contains(parent_id) {
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
    if visible.contains(&state.review.idx) {
        state.diff_offset = state
            .diff_offset
            .min(crate::panel::main::max_scroll_offset(state));
        return;
    }
    if let Some(first) = visible.first() {
        state.review.idx = *first;
    }
    state.diff_offset = state
        .diff_offset
        .min(crate::panel::main::max_scroll_offset(state));
}
