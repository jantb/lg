//! The diff pane's cursor over hunks, and over the commits of a branch log.
//!
//! A file, a folder or every change is shown as two diffs, the staged one over
//! the unstaged one, and the cursor names a hunk in one of them. `space` acts
//! on it according to which: a hunk under `== worktree ==` is staged, one
//! under `== staged (--cached) ==` is unstaged. `d` throws an unstaged hunk
//! away, after asking. A commit's hunks can be walked but not staged.
//!
//! A branch log gets the same cursor over its commits, for `C` to cherry-pick
//! the one it is on: the log is the one view that shows another branch's
//! commits, the Commits pane following the checked-out branch.
//!
//! The cursor is drawn on the hunk's `@@` line, or the commit's line of the
//! log, and the pane's title says which hunk of how many it is on.

use std::sync::Arc;

use crate::git::hunk::{HunkOp, HunkSide, ShownHunk, shown_hunks};
use crate::state::{AppState, DiffSource, HunkCursor, PendingAction};

/// Whether the text is lg's staged-over-worktree view, whose hunks can be
/// staged and unstaged.
fn has_sections(source: &DiffSource) -> bool {
    matches!(
        source,
        DiffSource::All | DiffSource::File(_) | DiffSource::Folder(_)
    )
}

/// Whether the pane is showing hunks the cursor can move over.
pub(super) fn hunks_shown(state: &AppState) -> bool {
    matches!(
        state.diff_source,
        DiffSource::All | DiffSource::File(_) | DiffSource::Folder(_) | DiffSource::Commit(_)
    )
}

/// Whether the pane is showing a branch log.
pub(super) fn log_shown(state: &AppState) -> bool {
    matches!(state.diff_source, DiffSource::Branch(_))
}

/// The hunks of the pane's text, read once per text.
pub(super) fn hunks(state: &AppState) -> Arc<Vec<ShownHunk>> {
    let version = state.diff_text_version;
    let sections = has_sections(&state.diff_source);
    // The key folds in whether the text has sections, so the same text shown
    // as a commit and as a file cannot share an answer.
    let key = version.wrapping_mul(2) | u64::from(sections);
    let mut cache = state.diff_hunks_cache.borrow_mut();
    if let Some((cached, hunks)) = cache.as_ref()
        && *cached == key
    {
        return Arc::clone(hunks);
    }
    // A hunk the cap on a long diff cut through is not whole, so it is not
    // offered: staging it would stage only the part that happened to fit.
    let hunks = Arc::new(if hunks_shown(state) {
        shown_hunks(&state.diff_text, sections)
            .into_iter()
            .filter(|shown| shown.hunk.is_complete())
            .collect()
    } else {
        Vec::new()
    });
    *cache = Some((key, Arc::clone(&hunks)));
    hunks
}

/// Which of `hunks` the cursor is on: the `index`th of its side, or the
/// nearest there is when that side has fewer — or none left at all, as after
/// staging the last unstaged hunk.
fn resolve(cursor: HunkCursor, hunks: &[ShownHunk]) -> Option<usize> {
    if hunks.is_empty() {
        return None;
    }
    let same_side: Vec<usize> = hunks
        .iter()
        .enumerate()
        .filter(|(_, hunk)| hunk.side == cursor.side)
        .map(|(at, _)| at)
        .collect();
    match same_side.last() {
        Some(last) => Some(same_side.get(cursor.index).copied().unwrap_or(*last)),
        None => Some(0),
    }
}

fn cursor_at(hunks: &[ShownHunk], at: usize) -> HunkCursor {
    let side = hunks[at].side;
    HunkCursor {
        side,
        index: hunks[..at].iter().filter(|hunk| hunk.side == side).count(),
    }
}

/// The hunk the cursor is on, with its place among the pane's hunks.
pub(super) fn current(state: &AppState) -> Option<(usize, ShownHunk)> {
    let hunks = hunks(state);
    let at = resolve(state.diff_hunk, &hunks)?;
    Some((at, hunks[at].clone()))
}

/// Rows of the diff drawn `width` wide where each hunk's `@@` line starts,
/// in order.
pub(super) fn hunk_rows(state: &AppState, width: u16) -> Vec<usize> {
    super::with_diff_rows(state, width, |rows| {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| {
                row.spans
                    .first()
                    .is_some_and(|span| span.content.starts_with("@@ "))
            })
            .map(|(at, _)| at)
            .collect()
    })
}

/// The commits of a branch log, as the line each starts on and its sha.
pub(super) fn log_commits(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(at, line)| {
            let rest = line.trim_start_matches(['*', '|', '/', '\\', ' ', '_', '.', '-']);
            let sha = rest.strip_prefix("commit ")?.split_whitespace().next()?;
            (sha.len() >= 4 && sha.chars().all(|ch| ch.is_ascii_hexdigit()))
                .then(|| (at, sha.to_string()))
        })
        .collect()
}

/// The rows of the drawn log each of `lines` starts on. The log is wrapped by
/// the paragraph that draws it, so the lines above count for every row they
/// wrap onto.
fn log_rows(state: &AppState, lines: &[usize]) -> Vec<usize> {
    let mut rows = Vec::with_capacity(lines.len());
    let mut wanted = lines.iter().peekable();
    let mut row = 0usize;
    for (at, text) in state.diff_text.lines().enumerate() {
        while wanted.next_if(|line| **line == at).is_some() {
            rows.push(row);
        }
        if wanted.peek().is_none() {
            break;
        }
        row += super::wrapped_line_count([text], state.diff_viewport_width);
    }
    rows
}

/// The rows of the pane each anchor — hunk or commit — starts on.
fn anchor_rows(state: &AppState) -> Vec<usize> {
    if log_shown(state) {
        let lines: Vec<usize> = log_commits(&state.diff_text)
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        log_rows(state, &lines)
    } else {
        hunk_rows(state, state.diff_viewport_width)
    }
}

/// The anchor the cursor is on, counted from the top.
fn current_anchor(state: &AppState) -> Option<usize> {
    if log_shown(state) {
        let count = log_commits(&state.diff_text).len();
        (count > 0).then(|| state.log_commit.min(count - 1))
    } else {
        current(state).map(|(at, _)| at)
    }
}

fn set_anchor(state: &mut AppState, at: usize) {
    if log_shown(state) {
        state.log_commit = at;
    } else {
        let hunks = hunks(state);
        if at < hunks.len() {
            state.diff_hunk = cursor_at(&hunks, at);
        }
    }
}

/// Move the cursor to the next or previous hunk (commit, in a log), and
/// scroll it into view with a line above it for context.
pub(super) fn step(state: &mut AppState, forward: bool) {
    let rows = anchor_rows(state);
    let Some(at) = current_anchor(state) else {
        let what = if log_shown(state) { "commits" } else { "hunks" };
        state.set_status(format!("no {what} to move between here"), false);
        return;
    };
    let count = rows.len().min(if log_shown(state) {
        log_commits(&state.diff_text).len()
    } else {
        hunks(state).len()
    });
    if count == 0 {
        return;
    }
    let next = if forward {
        (at + 1).min(count - 1)
    } else {
        at.saturating_sub(1)
    };
    set_anchor(state, next);
    let row = rows[next].saturating_sub(1);
    let max = super::max_scroll_offset(state) as usize;
    state.diff_offset = row.min(max).min(u16::MAX as usize) as u16;
}

/// Whether any of anchor `at`'s rows — from its own line to the next
/// anchor's — is on screen.
fn on_screen(state: &AppState, rows: &[usize], at: usize) -> bool {
    let top = state.diff_offset as usize;
    let bottom = top + (state.diff_viewport_height as usize).max(1);
    let start = rows[at];
    let end = rows.get(at + 1).copied().unwrap_or(usize::MAX);
    start < bottom && end > top
}

/// After scrolling, keep the cursor on screen: once the hunk it was on has
/// gone out of view, it moves to the one at the top of what is shown.
pub(super) fn follow_scroll(state: &mut AppState) {
    let rows = anchor_rows(state);
    let Some(at) = current_anchor(state) else {
        return;
    };
    if rows.is_empty() || at >= rows.len() || on_screen(state, &rows, at) {
        return;
    }
    let on_top = rows
        .iter()
        .rposition(|row| *row <= state.diff_offset as usize);
    set_anchor(state, on_top.unwrap_or(0));
}

/// After the text was read again, put cursor and screen back together. With
/// the pane focused the screen goes to the cursor — after staging a hunk, that
/// is the next one to stage — and otherwise the cursor comes to the screen.
pub(super) fn settle(state: &mut AppState) {
    let rows = anchor_rows(state);
    let Some(at) = current_anchor(state) else {
        return;
    };
    if rows.is_empty() || at >= rows.len() || on_screen(state, &rows, at) {
        return;
    }
    if state.focus == crate::state::Pane::Main {
        let max = super::max_scroll_offset(state) as usize;
        let row = rows[at].saturating_sub(1).min(max);
        state.diff_offset = row.min(u16::MAX as usize) as u16;
    } else {
        follow_scroll(state);
    }
}

/// The row of the diff drawn `width` wide to draw the cursor on.
pub(super) fn cursor_row(state: &AppState, width: u16) -> Option<usize> {
    let (at, _) = current(state)?;
    hunk_rows(state, width).get(at).copied()
}

/// The line of the branch log to draw the cursor on.
pub(super) fn cursor_log_line(state: &AppState) -> Option<usize> {
    let at = current_anchor(state)?;
    log_commits(&state.diff_text).get(at).map(|(line, _)| *line)
}

/// The pane's title addition: which hunk of how many the cursor is on.
pub(super) fn title_note(state: &AppState) -> Option<String> {
    if log_shown(state) {
        let commits = log_commits(&state.diff_text);
        let at = current_anchor(state)?;
        return Some(format!("commit {}/{}", at + 1, commits.len()));
    }
    let hunks = hunks(state);
    let (at, hunk) = current(state)?;
    Some(format!(
        "hunk {}/{} {}",
        at + 1,
        hunks.len(),
        hunk.side.label()
    ))
}

/// `space`: stage the hunk when it is unstaged, unstage it when it is staged.
pub(super) fn toggle_stage(state: &mut AppState) {
    let Some((_, hunk)) = actionable(state) else {
        return;
    };
    let op = match hunk.side {
        HunkSide::Worktree => HunkOp::Stage,
        HunkSide::Staged => HunkOp::Unstage,
        HunkSide::Committed => return,
    };
    state.pending_action = Some(PendingAction::ApplyHunk {
        op,
        hunk: Box::new(hunk),
    });
}

/// `d`: throw an unstaged hunk away, once the user has said yes.
pub(super) fn discard(state: &mut AppState) {
    let Some((_, hunk)) = actionable(state) else {
        return;
    };
    if hunk.side != HunkSide::Worktree {
        state.set_status(
            "only an unstaged hunk can be discarded \u{2014} space unstages this one first",
            false,
        );
        return;
    }
    if hunk.is_new {
        state.set_status(
            format!("{} is untracked: d in the Files pane deletes it", hunk.path),
            false,
        );
        return;
    }
    let changed: Vec<String> = hunk
        .hunk
        .lines
        .iter()
        .filter(|line| {
            matches!(
                line.kind,
                crate::git::patch::LineKind::Added | crate::git::patch::LineKind::Removed
            )
        })
        .take(6)
        .map(|line| line.to_patch_line())
        .collect();
    let detail = format!(
        "{}\n{}\n{}",
        hunk.path,
        hunk.hunk.header,
        changed.join("\n")
    );
    let question = format!("Discard this hunk of {} from the working tree?", hunk.path);
    state.confirm_action(
        "Discard hunk",
        question,
        detail,
        PendingAction::ApplyHunk {
            op: HunkOp::Discard,
            hunk: Box::new(hunk),
        },
    );
}

/// The hunk under the cursor, when it is one that can be staged or thrown
/// away; otherwise a status line saying why not. A cursor that is off screen
/// is brought to the screen first, so what is acted on is always in view.
fn actionable(state: &mut AppState) -> Option<(usize, ShownHunk)> {
    follow_scroll(state);
    if matches!(state.diff_source, DiffSource::Commit(_)) {
        state.set_status(
            "a commit's hunks are history \u{2014} select a file in Files to stage hunks",
            false,
        );
        return None;
    }
    if !has_sections(&state.diff_source) {
        state.set_status("no hunks to stage here", false);
        return None;
    }
    let found = current(state);
    if found.is_none() {
        state.set_status("no hunk here to stage", false);
    }
    found
}

/// `C` in a branch log: cherry-pick the commit the cursor is on onto the
/// checked-out branch, once the user has said yes.
pub(super) fn cherry_pick(state: &mut AppState) {
    let DiffSource::Branch(branch) = state.diff_source.clone() else {
        state.set_status(
            "cherry-pick works from a branch log: select a branch in Branches, then 0",
            false,
        );
        return;
    };
    if state.branch.as_deref() == Some(branch.as_str()) {
        state.set_status(
            format!("{branch} is the checked-out branch; pick another branch's log"),
            false,
        );
        return;
    }
    let commits = log_commits(&state.diff_text);
    let Some(at) = current_anchor(state) else {
        state.set_status("no commit under the cursor", false);
        return;
    };
    let (line, sha) = commits[at].clone();
    let subject = state
        .diff_text
        .lines()
        .skip(line + 1)
        .map(|text| text.trim_start_matches(['|', '/', '\\', ' ', '*']))
        .find(|text| {
            !text.is_empty()
                && !text.starts_with("Author:")
                && !text.starts_with("Date:")
                && !text.starts_with("Merge:")
        })
        .unwrap_or_default()
        .to_string();
    let onto = state.branch.clone().unwrap_or_else(|| "HEAD".to_string());
    state.confirm_reversible_action(
        "Cherry-pick",
        format!("Cherry-pick {sha} from {branch} onto {onto}?"),
        format!("{subject}\nA conflict opens the conflict editor; a aborts the cherry-pick there."),
        PendingAction::CherryPick { sha },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_graph_log_names_its_commits_by_the_line_they_start_on() {
        let log = "* commit 1a2b3c4 (HEAD -> feature)\n| Author: A <a@b>\n|\n|     one\n|\n* commit 5d6e7f8\n  Author: A <a@b>\n";
        let commits = log_commits(log);
        assert_eq!(
            commits,
            vec![(0, "1a2b3c4".to_string()), (5, "5d6e7f8".to_string())]
        );
    }
}
