use ratatui::layout::Rect;

use crate::{
    state::{AppState, Pane},
    ui,
};

pub(super) fn current_left_panel_heights(rects: &ui::LayoutRects) -> ui::LeftPanelHeights {
    [
        rects.status.height,
        rects.environments.height,
        rects.files.height,
        rects.branches.height,
        rects.commits.height,
    ]
}

fn left_panel_rect(rects: &ui::LayoutRects, idx: usize) -> Rect {
    match idx {
        0 => rects.status,
        1 => rects.environments,
        2 => rects.files,
        3 => rects.branches,
        4 => rects.commits,
        _ => rects.status,
    }
}

fn left_panel_total_height(rects: &ui::LayoutRects) -> u16 {
    rects
        .status
        .height
        .saturating_add(rects.environments.height)
        .saturating_add(rects.files.height)
        .saturating_add(rects.branches.height)
        .saturating_add(rects.commits.height)
}

pub(super) fn row_divider_pair_at(
    rects: &ui::LayoutRects,
    show_environments: bool,
    column: u16,
    row: u16,
) -> Option<(usize, usize)> {
    let in_left = column >= rects.status.x
        && column < rects.status.x.saturating_add(rects.status.width)
        && row >= rects.status.y
        && row < rects.footer.y;
    if !in_left {
        return None;
    }

    let pairs: &[(usize, usize)] = if show_environments {
        &[(0, 1), (1, 2), (2, 3), (3, 4)]
    } else {
        &[(0, 2), (2, 3), (3, 4)]
    };
    pairs.iter().copied().find(|(_, lower_idx)| {
        let lower = left_panel_rect(rects, *lower_idx);
        lower.height > 0 && (row == lower.y || row.saturating_add(1) == lower.y)
    })
}

pub(super) fn resize_left_panel_pair(
    state: &mut AppState,
    rects: &ui::LayoutRects,
    pair: (usize, usize),
    row: u16,
    show_environments: bool,
) {
    let total_height = left_panel_total_height(rects);
    let mut heights = ui::normalize_left_panel_heights(
        total_height,
        show_environments,
        Some(
            state
                .left_panel_heights
                .unwrap_or_else(|| current_left_panel_heights(rects)),
        ),
    );
    let (upper_idx, lower_idx) = pair;
    let upper = left_panel_rect(rects, upper_idx);
    let lower = left_panel_rect(rects, lower_idx);
    let pair_total = heights[upper_idx].saturating_add(heights[lower_idx]);
    let min_height = ui::left_panel_min_height(total_height, show_environments);
    if pair_total <= min_height.saturating_mul(2) {
        state.left_panel_heights = Some(heights);
        return;
    }

    let desired_upper = if row < lower.y {
        row.saturating_sub(upper.y).saturating_add(1)
    } else {
        row.saturating_sub(upper.y)
    };
    let upper_height = desired_upper
        .max(min_height)
        .min(pair_total.saturating_sub(min_height));
    heights[upper_idx] = upper_height;
    heights[lower_idx] = pair_total.saturating_sub(upper_height);
    state.left_panel_heights = Some(ui::normalize_left_panel_heights(
        total_height,
        show_environments,
        Some(heights),
    ));
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

pub(super) fn pane_at(rects: &ui::LayoutRects, column: u16, row: u16) -> Option<Pane> {
    [
        (Pane::Status, rects.status),
        (Pane::Files, rects.files),
        (Pane::Branches, rects.branches),
        (Pane::Commits, rects.commits),
        (Pane::Main, rects.main),
    ]
    .into_iter()
    .find_map(|(pane, rect)| rect_contains(rect, column, row).then_some(pane))
}

fn list_row_at(area: Rect, row: u16, len: usize) -> Option<usize> {
    if len == 0 || row <= area.y || row >= area.y.saturating_add(area.height).saturating_sub(1) {
        return None;
    }
    let idx = row.saturating_sub(area.y).saturating_sub(1) as usize;
    (idx < len).then_some(idx)
}

pub(super) fn select_mouse_row(
    state: &mut AppState,
    pane: Pane,
    rects: &ui::LayoutRects,
    row: u16,
) {
    match pane {
        Pane::Files => {
            let rows = state.tree_rows();
            if let Some(idx) = list_row_at(rects.files, row, rows.len()) {
                state.files_idx = idx;
            }
        }
        Pane::Branches => {
            if let Some(idx) = list_row_at(rects.branches, row, state.branches.len()) {
                state.branches_idx = idx;
            }
        }
        Pane::Commits => {
            if let Some(idx) = list_row_at(rects.commits, row, state.commits.len()) {
                state.commits_idx = idx;
            }
        }
        Pane::Status | Pane::Main => {}
    }
}

pub(super) fn scroll_list(
    state: &mut AppState,
    pane: Pane,
    scroll_down: bool,
    amount: usize,
) -> bool {
    match pane {
        Pane::Files => {
            let len = state.tree_rows().len();
            scroll_index(&mut state.files_idx, len, scroll_down, amount)
        }
        Pane::Branches => scroll_index(
            &mut state.branches_idx,
            state.branches.len(),
            scroll_down,
            amount,
        ),
        Pane::Commits => scroll_index(
            &mut state.commits_idx,
            state.commits.len(),
            scroll_down,
            amount,
        ),
        Pane::Status | Pane::Main => false,
    }
}

fn scroll_index(idx: &mut usize, len: usize, scroll_down: bool, amount: usize) -> bool {
    let old = *idx;
    if len == 0 {
        *idx = 0;
        return old != *idx;
    }

    let current = (*idx).min(len - 1);
    *idx = if scroll_down {
        current.saturating_add(amount).min(len - 1)
    } else {
        current.saturating_sub(amount)
    };
    old != *idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Branch, Commit, FileEntry};

    fn branch(name: &str) -> Branch {
        Branch {
            name: name.into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
        }
    }

    fn commit(sha: &str) -> Commit {
        Commit {
            sha: sha.into(),
            author: "Test Author".into(),
            author_short: "TA".into(),
            graph: "* ".into(),
            is_first_parent: true,
            parent_count: 1,
            subject: "change".into(),
        }
    }

    #[test]
    fn scroll_list_clamps_branches_to_bounds() {
        let mut state = AppState::new();
        state.branches = vec![branch("one"), branch("two"), branch("three")];
        state.branches_idx = 1;

        assert!(scroll_list(&mut state, Pane::Branches, true, 99));
        assert_eq!(state.branches_idx, 2);
        assert!(!scroll_list(&mut state, Pane::Branches, true, 99));
        assert_eq!(state.branches_idx, 2);

        assert!(scroll_list(&mut state, Pane::Branches, false, 99));
        assert_eq!(state.branches_idx, 0);
        assert!(!scroll_list(&mut state, Pane::Branches, false, 99));
        assert_eq!(state.branches_idx, 0);
    }

    #[test]
    fn scroll_list_clamps_empty_lists_to_zero() {
        let mut state = AppState::new();
        state.commits_idx = 42;

        assert!(scroll_list(&mut state, Pane::Commits, true, 3));
        assert_eq!(state.commits_idx, 0);
        assert!(!scroll_list(&mut state, Pane::Commits, false, 3));
        assert_eq!(state.commits_idx, 0);
    }

    #[test]
    fn scroll_list_moves_files_and_commits() {
        let mut state = AppState::new();
        state.files = vec![
            FileEntry {
                path: "src/a.rs".into(),
                x: ' ',
                y: 'M',
            },
            FileEntry {
                path: "src/b.rs".into(),
                x: ' ',
                y: 'M',
            },
            FileEntry {
                path: "tests/c.rs".into(),
                x: 'A',
                y: ' ',
            },
        ];
        let file_rows = state.tree_rows().len();

        assert!(scroll_list(&mut state, Pane::Files, true, 99));
        assert_eq!(state.files_idx, file_rows - 1);
        assert!(scroll_list(&mut state, Pane::Files, false, 99));
        assert_eq!(state.files_idx, 0);

        state.commits = vec![commit("a"), commit("b"), commit("c")];
        assert!(scroll_list(&mut state, Pane::Commits, true, 2));
        assert_eq!(state.commits_idx, 2);
        assert!(scroll_list(&mut state, Pane::Commits, false, 1));
        assert_eq!(state.commits_idx, 1);
    }
}
