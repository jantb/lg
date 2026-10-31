//! Moving through the walk: from step to step, finding to finding, and line
//! to line.

use crate::git::guided::GuidedFile;
use crate::state::{AppState, Modal};

use super::notes::open_summary;
use super::{Guided, Mode, OVERVIEW, OpenFinding, Source};

/// Move to step `index`, marking the one being left as reviewed when moving
/// forward past it.
pub(super) fn go_to(state: &mut AppState, index: usize, mark_left: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if mark_left && let Some(step) = guided.step() {
        let key = step.key();
        if !guided.progress.reviewed.contains(&key) {
            guided.progress.reviewed.push(key);
            if let Err(message) = guided.save() {
                state.set_status(message, true);
                return;
            }
        }
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    guided.current = index.min(guided.final_step());
    guided.cursor = guided.step().map(GuidedFile::first_changed).unwrap_or(0);
    guided.side_scroll = 0;
    guided.final_scroll.set(0);
    guided.list_scroll = None;
    if guided.on_final() {
        let noun = if guided.source.is_pull_request() {
            "s submits the review"
        } else {
            "s shows every note"
        };
        state.set_status(
            format!(
                "final review of the whole change \u{2014} \u{2193}/\u{2191} pick a finding, Enter go to it, f fix, C chat, {noun}"
            ),
            false,
        );
    }
}

/// The next file in the walk's order; past the last is the final review, and
/// past that every note.
pub(super) fn next_step(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.on_final() {
        open_summary(state);
        return;
    }
    let next = guided.step_after(guided.current);
    go_to(state, next, true);
}

/// The next walk mode. Each orders the same files from the same plan, so
/// switching keeps every note, tick and reviewed mark.
pub(super) fn switch_walk(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    guided.progress.walk = guided.progress.walk.next();
    let walk = guided.progress.walk;
    let waiting = if guided.plan().is_none() {
        " (in path order until the plan is read)"
    } else {
        ""
    };
    match guided.save() {
        Ok(()) => state.set_status(
            format!(
                "walk by {}: {}{waiting}",
                walk.label().to_lowercase(),
                walk.describe()
            ),
            false,
        ),
        Err(message) => state.set_status(message, true),
    }
}

/// The next open finding, worst first: its file on screen, the cursor on its
/// line. Past the last is the final review, and past that every note.
pub(super) fn next_issue(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.on_final() {
        open_summary(state);
        return;
    }
    let all = guided.findings_in_order();
    let from = finding_here(guided, &all).map_or(0, |at| at + 1);
    let next = all[from..]
        .iter()
        .find(|(_, ticked)| !ticked)
        .map(|(open, _)| open.clone());
    match next {
        Some(finding) => go_to_finding(state, &finding),
        None => {
            go_final(state);
            let read = state.guided.as_deref().map_or(0, |guided| {
                guided
                    .steps
                    .iter()
                    .filter(|step| guided.is_read(step))
                    .count()
            });
            let total = state.guided.as_deref().map_or(0, Guided::total);
            state.set_status(
                format!("no more open findings in the {read} of {total} file(s) read"),
                false,
            );
        }
    }
}

pub(super) fn previous_issue(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    let all = guided.findings_in_order();
    let before = match finding_here(guided, &all) {
        Some(at) => at,
        None if guided.on_final() => all.len(),
        None => 0,
    };
    let previous = all[..before]
        .iter()
        .rev()
        .find(|(_, ticked)| !ticked)
        .map(|(open, _)| open.clone());
    match previous {
        Some(finding) => go_to_finding(state, &finding),
        None => go_to(state, OVERVIEW, false),
    }
}

/// The finding the cursor is on, ticked or not.
fn finding_here(guided: &Guided, all: &[(OpenFinding, bool)]) -> Option<usize> {
    all.iter()
        .position(|(open, _)| open.step == guided.current && open.at == Some(guided.cursor))
}

pub(super) fn go_to_finding(state: &mut AppState, finding: &OpenFinding) {
    go_to(state, finding.step, false);
    if let Some(guided) = state.guided.as_mut()
        && let Some(at) = finding.at
    {
        guided.cursor = at;
    }
}

pub(super) fn go_final(state: &mut AppState) {
    if let Some(last) = state.guided.as_deref().map(Guided::final_step) {
        go_to(state, last, false);
    }
}

/// Scrolls the whole changeset on the final step, as the cursor scrolls a file.
pub(super) fn scroll_final(state: &mut AppState, by: i32) {
    if let Some(guided) = state.guided.as_mut() {
        let from = guided.final_scroll.get().min(guided.final_max.get());
        guided.final_scroll.set(if by >= 0 {
            from.saturating_add(by as u16)
        } else {
            from.saturating_sub(by.unsigned_abs() as u16)
        });
    }
}

pub(super) fn previous_step(state: &mut AppState) {
    let Some(previous) = state
        .guided
        .as_deref()
        .map(|guided| guided.step_before(guided.current))
    else {
        return;
    };
    go_to(state, previous, false);
}

pub(super) fn next_unreviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    let order = guided.walk_order();
    let here = order
        .iter()
        .position(|&step| step == guided.current)
        .map_or(0, |at| at + 1);
    let after = order[here..]
        .iter()
        .chain(&order[..here])
        .copied()
        .find(|&step| !guided.is_reviewed(&guided.steps[step - 1]));
    match after {
        Some(index) => go_to(state, index, false),
        None => state.set_status("every file is reviewed", false),
    }
}

pub(super) fn move_cursor(state: &mut AppState, down: bool) {
    let mode = state.diff_view_mode;
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.on_final() {
        scroll_final(state, if down { 1 } else { -1 });
        return;
    }
    if guided.step().is_none() {
        guided.side_scroll = if down {
            guided.side_scroll.saturating_add(1)
        } else {
            guided.side_scroll.saturating_sub(1)
        };
        return;
    }
    let order = guided.visit_order(mode);
    let here = order
        .iter()
        .position(|&at| at == guided.cursor)
        .unwrap_or(0);
    let next = if down {
        here + 1
    } else {
        here.saturating_sub(1)
    };
    if let Some(&at) = order.get(next) {
        guided.cursor = at;
    }
}

/// Until the model has read the step there is nothing to jump to, so it moves a line.
pub(super) fn next_finding(state: &mut AppState) {
    let (ai, mode) = (state.ai_assist, state.diff_view_mode);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.on_final() {
        pick_final(state, true);
        return;
    }
    if guided.step().is_none() || guided.still_reading(ai) {
        move_cursor(state, true);
        return;
    }
    let (order, flagged) = (guided.visit_order(mode), guided.flagged_lines(mode));
    let place = |at: usize| order.iter().position(|&o| o == at);
    let here = place(guided.cursor);
    if let Some(at) = flagged.into_iter().find(|&at| place(at) > here) {
        guided.cursor = at;
        return;
    }
    let left = guided.current;
    next_step(state);
    land_on_finding(state, left, true);
}

pub(super) fn previous_finding(state: &mut AppState) {
    let (ai, mode) = (state.ai_assist, state.diff_view_mode);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.on_final() {
        pick_final(state, false);
        return;
    }
    if guided.step().is_none() || guided.still_reading(ai) {
        move_cursor(state, false);
        return;
    }
    let (order, flagged) = (guided.visit_order(mode), guided.flagged_lines(mode));
    let place = |at: usize| order.iter().position(|&o| o == at);
    let here = place(guided.cursor);
    if let Some(at) = flagged.into_iter().rev().find(|&at| place(at) < here) {
        guided.cursor = at;
        return;
    }
    let left = guided.current;
    previous_step(state);
    land_on_finding(state, left, false);
}

/// Pick the next or previous finding the final review cites; with none to
/// pick, the whole change scrolls a line instead.
fn pick_final(state: &mut AppState, down: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(last) = guided.final_cited().len().checked_sub(1) else {
        scroll_final(state, if down { 1 } else { -1 });
        return;
    };
    let here = guided.final_pick.min(last);
    guided.final_pick = if down {
        (here + 1).min(last)
    } else {
        here.saturating_sub(1)
    };
    guided.final_reveal.set(true);
}

/// Open the picked finding's file with the cursor on its line.
pub(super) fn go_to_picked(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    match (guided.picked_open(), guided.picked()) {
        (Some(open), _) => go_to_finding(state, &open),
        (None, Some((path, _))) => {
            state.set_status(format!("{path} is not part of the change"), false)
        }
        (None, None) => state.set_status("the final review has no findings to go to", false),
    }
}

fn land_on_finding(state: &mut AppState, left: usize, first: bool) {
    let mode = state.diff_view_mode;
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.current == left || !matches!(guided.mode, Mode::Browse) {
        return;
    }
    let flagged = guided.flagged_lines(mode);
    if let Some(&at) = if first {
        flagged.first()
    } else {
        flagged.last()
    } {
        guided.cursor = at;
    }
}

/// Shared with the diff pane.
pub(super) fn toggle_layout(state: &mut AppState) {
    state.diff_view_mode = state.diff_view_mode.toggled();
    state.set_status(format!("showing {}", state.diff_view_mode.label()), false);
}

/// Leave the walk. Its progress is already on disk; the commentary stays in
/// memory, so coming back the same session is instant.
pub(super) fn leave(state: &mut AppState) {
    let back = match state.guided.as_ref().map(|g| &g.source) {
        Some(Source::PullRequest { .. }) => Modal::GitHub,
        _ => Modal::None,
    };
    if let Some(guided) = state.guided.as_mut() {
        guided.mode = Mode::Browse;
    }
    state.modal = back;
}
