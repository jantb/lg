//! Where the modal and its parts sit on screen, and what a click there lands
//! on.

use ratatui::layout::{Constraint, Position, Rect};

use crate::github::PrFilter;
use crate::panel::github::Tab;
use crate::panel::github::lookup::visible_repos;
use crate::state::AppState;
use crate::ui;

use super::header::header_spans;
use super::{ROWS, inbox};

/// Nearly the whole screen, so long titles and descriptions fit, but clear of
/// the footer, where the status line reports what the last action did, and
/// no wider than a line is still comfortable to read across.
pub(super) fn modal_area(area: Rect) -> Rect {
    ui::centered(
        area,
        area.width.saturating_sub(4).min(220),
        area.height.saturating_sub(2),
    )
}

pub(super) fn too_small(inner: Rect) -> bool {
    inner.width < 40 || inner.height < 8
}

/// What a click on the header asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::panel::github) enum HeaderClick {
    Tab(Tab),
    /// A pull request filter to show.
    Filter(PrFilter),
    /// The filter shown when there is no room for every choice, which steps
    /// to the next one.
    NextFilter,
    /// Whether drafts are shown, which flips.
    Drafts,
}

/// What the header holds at `column`, `row`, measured the way [`render`]
/// lays it out.
pub(in crate::panel::github) fn header_click(
    state: &AppState,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<HeaderClick> {
    let inner = ui::modal_inner(modal_area(area));
    if too_small(inner) {
        return None;
    }
    let header = ui::modal_row_areas(inner, &ROWS).0[0];
    if row != header.y || column < header.x {
        return None;
    }
    let mut x = header.x as usize;
    for (span, click) in header_spans(state, header.width) {
        x += span.width();
        if (column as usize) < x {
            return click;
        }
    }
    None
}

/// How a tab splits its body between the list and the details.
pub(super) fn columns(tab: Tab) -> &'static [Constraint] {
    match tab {
        Tab::PullRequests => &[Constraint::Percentage(42), Constraint::Min(20)],
        Tab::Inbox | Tab::Repositories => &[Constraint::Percentage(50), Constraint::Min(20)],
    }
}

/// The search line above the repository list, and the list under it.
pub(super) fn repo_list_areas(area: Rect) -> [Rect; 2] {
    ratatui::layout::Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(area)
}

/// The index, in the tab's list, of the entry drawn at `column`, `row`,
/// measured the way [`render`] lays the list out and scrolls it.
pub(in crate::panel::github) fn list_click(
    state: &AppState,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<usize> {
    let inner = ui::modal_inner(modal_area(area));
    if too_small(inner) {
        return None;
    }
    let body = ui::modal_row_areas(inner, &ROWS).0[1];
    let gh = &state.github;
    let pane = ui::modal_column_areas(body, columns(gh.tab)).0[0];
    let pane = match gh.tab {
        Tab::Repositories => repo_list_areas(pane)[1],
        _ => pane,
    };
    if !pane.contains(Position::new(column, row)) {
        return None;
    }
    let visible = pane.height as usize;
    let line = (row - pane.y) as usize;
    let at = |selected: usize| selected.saturating_sub(visible.saturating_sub(1)) + line;
    match gh.tab {
        Tab::Inbox => inbox::item_at(state, at(inbox::selected_row(state))),
        Tab::PullRequests => Some(at(gh.pr_idx)).filter(|&idx| idx < gh.prs.len()),
        Tab::Repositories => Some(at(gh.repo_idx)).filter(|&idx| idx < visible_repos(gh).len()),
    }
}
