//! The review tree: what each node is, and how the pane reads it.

use crate::state::{AppState, DiffViewMode};

mod draw;
mod keys;
mod layout;

pub(super) use draw::render;
pub(super) use keys::{handle_key, select_mouse_row, selected_open_path};
pub(super) use layout::render_line_count;

use draw::*;
use keys::*;
use layout::*;

fn side_by_side_diff_enabled(state: &AppState) -> bool {
    state.diff_view_mode == DiffViewMode::SideBySide
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

fn review_indent(depth: u16) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{}└─", "  │ ".repeat(depth.saturating_sub(1) as usize))
    }
}

fn path_from_review_title(title: &str) -> Option<String> {
    let path = review_node_path(title)?;
    super::is_supported_source_path(path).then(|| path.to_string())
}

fn is_test_review_node(title: &str) -> bool {
    title.starts_with("tests/")
        || title.contains("/tests/")
        || title.contains(" in tests/")
        || title.contains(" in src/test/")
        || title.starts_with("src/test/")
        || title.contains("/src/test/")
}
