//! The stash lg takes on the way into a flow, and puts back on the way out.

use anyhow::Result;

use super::super::{head_branch, run};
use super::*;

/// What lg calls the stashes it takes on the way into a flow. A flow that
/// fails puts its own stash back and a conflict says where it is, so the
/// message has to still be recognisable long after it was written.
pub(super) const AUTO_STASH_MERGE_MAIN: &str = "lg flow: auto-stash before merging main";

pub(super) const AUTO_STASH_RELEASE: &str = "lg flow: auto-stash before release";

pub(super) const AUTO_STASH_NEW_BRANCH: &str = "lg flow: auto-stash before branch creation";

const AUTO_STASH_TAGS: [&str; 2] = ["lg flow: auto-stash", "lg: auto-stash"];

pub(super) fn stash_before_branch_change(target: &str, message: &str) -> Result<bool> {
    if head_branch().is_ok_and(|current| current == target) {
        return Ok(false);
    }
    stash_uncommitted_changes(message)
}

pub(super) fn stash_uncommitted_changes(message: &str) -> Result<bool> {
    let stashed = has_uncommitted_changes()?;
    if stashed {
        run(&["stash", "push", "-u", "-m", message])?;
    }
    Ok(stashed)
}

pub(super) fn pop_stash_if_needed(stashed: bool) -> Result<()> {
    if stashed {
        run(&["stash", "pop"])?;
    }
    Ok(())
}

pub(super) fn pop_stash_with_index_if_needed(stashed: bool) -> Result<()> {
    if stashed {
        run(&["stash", "pop", "--index"])?;
    }
    Ok(())
}

/// Undo the auto-stash a failed flow took, and say in the error what became
/// of it. Leaving it stashed empties the checkout the user was working in and
/// nothing on screen says why, which is how a flow that stopped half-way looks
/// like lost work.
///
/// A conflicted merge is the one case the stash cannot come back on top of, so
/// that error carries where the work is instead; the conflict modal is showing
/// the whole message by then.
pub(super) fn restore_auto_stash_after_failure(
    err: anyhow::Error,
    stashed: bool,
    message: &str,
    branch: &str,
) -> anyhow::Error {
    if !stashed {
        return err;
    }
    // The flow may have popped it already and failed on a later step.
    if !matches!(newest_stash_subject(), Some(subject) if subject.contains(message)) {
        return err;
    }
    if merge_or_rebase_in_progress() {
        return anyhow::anyhow!(
            "{err:#}\nyour uncommitted changes are stashed as \"{message}\"; git stash pop brings them back once this is settled"
        );
    }
    // The flow may have stopped on another branch, and the stash belongs to
    // the one it started on.
    if head_branch().map(|head| head != branch).unwrap_or(true)
        && let Err(back) = run(&["checkout", branch])
    {
        return anyhow::anyhow!(
            "your uncommitted changes are stashed as \"{message}\" and {branch} could not be checked out again: {back:#}\n{err:#}"
        );
    }
    match pop_stash_with_index_if_needed(true) {
        Ok(()) => anyhow::anyhow!("{err:#}\nrestored your uncommitted changes"),
        Err(pop) => anyhow::anyhow!(
            "your uncommitted changes are stashed as \"{message}\" and could not be restored: {pop:#}\n{err:#}"
        ),
    }
}

/// Bring back what a flow stashed before it ran into the conflict now being
/// validated. Only lg's own stash is popped, and only while it is the newest
/// one, so a stash the user made themselves is never pulled out from under
/// them.
pub(super) fn restore_auto_stash_after_conflict() -> Option<String> {
    let subject = newest_stash_subject()?;
    if !AUTO_STASH_TAGS.iter().any(|tag| subject.contains(tag)) {
        return None;
    }
    match pop_stash_with_index_if_needed(true) {
        Ok(()) => Some(format!("restored \"{subject}\"")),
        Err(err) => Some(format!("could not restore \"{subject}\": {err:#}")),
    }
}

fn newest_stash_subject() -> Option<String> {
    let out = run(&["stash", "list", "-1", "--format=%gs"]).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let subject = text.lines().next()?.trim().to_string();
    (!subject.is_empty()).then_some(subject)
}
