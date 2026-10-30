//! Opening the modal, and loading pull requests, repositories and owners
//! into it as their answers come in.

use crate::state::{AppState, Modal};

use super::forms::start_create;
use super::lookup::{is_mine, visible_repos};
use super::work::take_answer;
use super::{GitHub, Mode, Tab, inbox, work};

pub fn open(state: &mut AppState) {
    let dir = crate::git::active_repo();
    if state.github.prs_dir != dir {
        state.github.prs.clear();
        state.github.repo = None;
        state.github.pr_idx = 0;
        state.github.prs_error = None;
    }
    state.github.mode = Mode::Browse;
    // With no checkout there are no pull requests to show, and cloning one is
    // the likely reason lg was opened here.
    state.github.tab = inbox::opening_tab(state).unwrap_or(if state.repo_root.is_some() {
        Tab::PullRequests
    } else {
        Tab::Repositories
    });
    state.modal = Modal::GitHub;
    match state.github.tab {
        Tab::Inbox => {}
        Tab::PullRequests => load_pull_requests(state),
        Tab::Repositories => load_repositories(state),
    }
}

/// Open the modal straight into the new pull request form for the branch
/// checked out here.
pub fn open_create(state: &mut AppState) {
    if state.repo_root.is_none() {
        state.set_status("lg is not pointed at a Git checkout", false);
        return;
    }
    open(state);
    state.github.create_on_load = true;
}

/// Read the pull request list again, and the repository's details if this
/// checkout has not had them yet.
pub fn load_pull_requests(state: &mut AppState) {
    if state.repo_root.is_none() {
        state.github.prs_error =
            Some("lg is not pointed at a Git checkout; Tab lists repositories to clone".into());
        return;
    }
    let filter = state.github.filter;
    let hide_drafts = state.github.hide_drafts;
    let need_info = state.github.repo.is_none();
    state.github.prs_loading = Some(work::pull_requests(filter, hide_drafts, need_info));
    state.github.prs_dir = crate::git::active_repo();
    state.github.prs_error = None;
    // Who the user is, so their own pull requests can be told apart.
    if state.github.owners.is_none() {
        load_owners(state);
    }
}

pub fn load_repositories(state: &mut AppState) {
    let owner = state.github.owner.clone();
    state.github.repos_loading = Some(work::repositories(owner));
    state.github.repos_error = None;
    if state.github.owners.is_none() {
        load_owners(state);
    }
}

/// Read which organizations the user belongs to, unless that is already
/// under way.
pub(super) fn load_owners(state: &mut AppState) {
    if state.github.owners_loading.is_some() {
        return;
    }
    state.github.owners_loading = Some(work::owners());
}

/// List the repositories of the next owner along — or the previous one.
pub(super) fn step_owner(state: &mut AppState, forward: bool) {
    let choices = state.github.owner_choices();
    if choices.len() < 2 {
        let why = if state.github.owners_loading.is_some() {
            "still reading your organizations"
        } else {
            "you belong to no organization gh can see; `gh auth refresh -s read:org` if you should"
        };
        state.set_status(why, false);
        return;
    }
    let len = choices.len();
    let idx = choices
        .iter()
        .position(|owner| *owner == state.github.owner)
        .unwrap_or(0);
    let next = if forward {
        (idx + 1) % len
    } else {
        (idx + len - 1) % len
    };
    state.github.owner = choices[next].clone();
    // The last owner's repositories must not be what Enter clones while this
    // one's are still on their way.
    state.github.repos.clear();
    state.github.repo_idx = 0;
    load_repositories(state);
}

/// A GitHub action has finished: what the lists show may no longer be true.
/// A review given, say, is a review request answered.
pub fn after_action(state: &mut AppState) {
    if state.modal == Modal::GitHub {
        load_pull_requests(state);
    }
    inbox::reload(state);
}

/// Take in whatever the lists being read have sent back.
pub fn poll(state: &mut AppState) {
    inbox::poll(state);
    let gh = &mut state.github;
    let mut create = false;
    if let Some(result) = take_answer(&mut gh.prs_loading) {
        match result {
            Ok((info, prs)) => {
                if info.is_some() {
                    gh.repo = info;
                }
                gh.prs = prs;
                gh.pr_idx = gh.pr_idx.min(gh.prs.len().saturating_sub(1));
                gh.prs_error = None;
                put_mine_first(gh);
                create = std::mem::take(&mut gh.create_on_load);
            }
            Err(err) => {
                gh.prs_error = Some(err);
                gh.create_on_load = false;
            }
        }
    }
    if let Some(result) = take_answer(&mut gh.repos_loading) {
        match result {
            Ok(repos) => {
                gh.repos = repos;
                gh.repos_error = None;
            }
            Err(err) => gh.repos_error = Some(err),
        }
    }
    let owners = take_answer(&mut gh.owners_loading);
    match owners {
        Some(Ok(owners)) => {
            state.github.owners = Some(owners);
            put_mine_first(&mut state.github);
        }
        // Only the organizations are missing; the user's own list stands.
        Some(Err(err)) => {
            state.set_status(format!("could not read your organizations: {err}"), true)
        }
        None => {}
    }
    let visible = visible_repos(&state.github).len();
    state.github.repo_idx = state.github.repo_idx.min(visible.saturating_sub(1));
    if create && state.modal == Modal::GitHub && state.github.mode == Mode::Browse {
        start_create(state);
    }
}

/// The user's own pull requests go to the top, in the order GitHub listed
/// them; the selection stays on the same pull request.
fn put_mine_first(gh: &mut GitHub) {
    let selected = gh.prs.get(gh.pr_idx).map(|pr| pr.number);
    let mut prs = std::mem::take(&mut gh.prs);
    prs.sort_by_key(|pr| !is_mine(gh, pr));
    gh.prs = prs;
    if let Some(idx) = selected.and_then(|n| gh.prs.iter().position(|pr| pr.number == n)) {
        gh.pr_idx = idx;
    }
}
