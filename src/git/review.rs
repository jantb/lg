//! The change a review reads: this branch against main, or what is not pushed.

use anyhow::Result;

use super::{head_branch, preferred_commit_ref, run};

mod category;
mod collect;
pub(crate) mod language;

use collect::{branch_review_commits, worktree_review_diff};

pub use category::is_test_path;
pub use language::Language;

/// The branch's whole change against main, as the review reads it: the working
/// tree against the merge-base, untracked files included and generated files
/// suppressed, so an edit made a moment ago is part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchDiff {
    pub branch: String,
    pub base_ref: String,
    /// One line per commit on the branch: short hash, subject, compact body.
    pub commits: Vec<String>,
    pub diff: String,
}

pub fn branch_diff_against_main() -> Result<BranchDiff> {
    let (branch, base_ref, merge_base) = review_bases()?;
    let diff_base = if merge_base.is_empty() {
        base_ref.as_str()
    } else {
        merge_base.as_str()
    };
    Ok(BranchDiff {
        commits: branch_review_commits(&base_ref)?,
        diff: worktree_review_diff(diff_base)?,
        branch,
        base_ref,
    })
}

/// What has not been pushed yet: the working tree against where the branch's
/// upstream and `HEAD` diverged, so unpushed commits and uncommitted edits
/// alike. A branch that was never pushed has all of its change unpushed, so
/// it is read against main as [`branch_diff_against_main`] reads it.
pub fn unpushed_diff() -> Result<BranchDiff> {
    let branch = head_branch().unwrap_or_else(|_| "HEAD".to_string());
    let Some(upstream) = pushed_ref(&branch) else {
        return branch_diff_against_main();
    };
    let merge_base = run(&["merge-base", &upstream, "HEAD"])
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let diff_base = if merge_base.is_empty() {
        upstream.as_str()
    } else {
        merge_base.as_str()
    };
    Ok(BranchDiff {
        commits: branch_review_commits(&upstream)?,
        diff: worktree_review_diff(diff_base)?,
        branch,
        base_ref: upstream,
    })
}

/// Git's empty tree: the codebase against it is every file, each read as added.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The whole codebase as it stands in the working tree, untracked files
/// included and generated files suppressed: what a review of all of it reads.
pub fn codebase_diff() -> Result<BranchDiff> {
    Ok(BranchDiff {
        branch: head_branch().unwrap_or_else(|_| "HEAD".to_string()),
        base_ref: String::new(),
        commits: Vec::new(),
        diff: worktree_review_diff(EMPTY_TREE)?,
    })
}

/// Where `branch` was last pushed: its upstream, else the branch of the same
/// name on the configured remote.
fn pushed_ref(branch: &str) -> Option<String> {
    let upstream = run(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
    ])
    .ok()
    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    .filter(|upstream| !upstream.is_empty());
    let same_name = format!("{}/{branch}", crate::preferences::remote());
    upstream.or_else(|| {
        let commit = format!("{same_name}^{{commit}}");
        run(&["rev-parse", "--verify", "--quiet", &commit])
            .ok()
            .map(|_| same_name)
    })
}

/// The branch on screen, the main ref it is reviewed against, and where the
/// two diverged (empty when they share no history).
fn review_bases() -> Result<(String, String, String)> {
    let configured_base = crate::preferences::base_branch();
    let configured_remote = crate::preferences::remote();
    let base_ref = preferred_commit_ref(
        &format!("{configured_remote}/{configured_base}"),
        configured_base.as_str(),
    )
    .ok_or_else(|| {
        anyhow::anyhow!("could not find {configured_remote}/{configured_base} or {configured_base}")
    })?;
    let branch = head_branch().unwrap_or_else(|_| "HEAD".to_string());

    let merge_base = run(&["merge-base", &base_ref, "HEAD"])
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    Ok((branch, base_ref, merge_base))
}

fn truncate_review_text(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_review_text_appends_ellipsis_when_cut() {
        assert_eq!(truncate_review_text("short", 10), "short");
        assert_eq!(truncate_review_text("0123456789xyz", 10), "0123456789...");
    }
}
