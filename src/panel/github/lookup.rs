//! What the modal is showing: the selected pull request, the repositories on
//! show, and where a clone would go.

use std::path::{Path, PathBuf};

use crate::github::{PullRequest, Repository};
use crate::state::AppState;

use super::GitHub;

pub fn selected_pr(state: &AppState) -> Option<&PullRequest> {
    state.github.prs.get(state.github.pr_idx)
}

/// The repositories the query leaves, in the order GitHub listed them.
pub fn visible_repos(gh: &GitHub) -> Vec<&Repository> {
    let query = gh.query.trim().to_lowercase();
    gh.repos
        .iter()
        .filter(|repo| {
            query.is_empty()
                || repo.name.to_lowercase().contains(&query)
                || repo.description.to_lowercase().contains(&query)
        })
        .collect()
}

/// What Enter on the repositories tab would clone: the highlighted
/// repository, or failing that whatever was typed, when it names one.
pub fn clone_request(state: &AppState) -> Option<String> {
    let visible = visible_repos(&state.github);
    if let Some(repo) = visible.get(state.github.repo_idx) {
        return Some(repo.name.clone());
    }
    let typed = state.github.query.trim();
    typed.contains('/').then(|| typed.to_string())
}

/// Where clones go: into the workspace when lg manages one, otherwise next to
/// the checkout on screen — never inside it.
pub fn clone_parent(state: &AppState) -> Option<PathBuf> {
    if let Some(workspace) = &state.workspace_root {
        return Some(PathBuf::from(workspace));
    }
    let checkout = state
        .worktrees
        .iter()
        .find(|worktree| worktree.is_main)
        .map(|worktree| worktree.path.clone())
        .or_else(|| state.repo_root.clone())?;
    Path::new(&checkout).parent().map(Path::to_path_buf)
}

pub fn clone_destination(state: &AppState) -> Option<PathBuf> {
    crate::github::clone_dir(&clone_parent(state)?, &clone_request(state)?)
}

/// Whether the signed-in user opened `pr`. Unknown until their login is read.
pub fn is_mine(gh: &GitHub, pr: &PullRequest) -> bool {
    gh.owners
        .as_ref()
        .is_some_and(|owners| !owners.login.is_empty() && owners.login == pr.author)
}

/// Whether `pr` is the pull request for the branch checked out here.
pub fn is_current_branch(state: &AppState, pr: &PullRequest) -> bool {
    pr.head_owner.is_none() && state.branch.as_deref() == Some(pr.head.as_str())
}

/// Where a pull request checked out into a worktree goes: beside the main
/// worktree, named after its branch, the same as `n` would put it.
pub(super) fn worktree_path_for(state: &AppState, pr: &PullRequest) -> Option<String> {
    let main = state
        .worktrees
        .iter()
        .find(|worktree| worktree.is_main)
        .map(|worktree| worktree.path.clone())
        .or_else(|| state.repo_root.clone())?;
    let branch = match &pr.head_owner {
        Some(owner) => format!("{owner}-{}", pr.head),
        None => pr.head.clone(),
    };
    Some(
        crate::git::default_worktree_path(Path::new(&main), &branch)
            .to_string_lossy()
            .into_owned(),
    )
}
