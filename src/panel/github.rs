//! GitHub from inside lg: the pull requests of the checkout on screen — check
//! one out, review it, merge it, open a new one — and the repositories of the
//! user or one of their organizations, to clone one.
//!
//! Everything goes through `gh` (see [`crate::github`]). Lists are read on a
//! thread of their own and picked up by [`poll`]; anything that changes
//! something is handed to the app as a [`PendingAction::GitHub`], so it runs
//! and reports like every other operation.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::github::{
    MergeMethod, NewPullRequest, Owners, PrFilter, PullRequest, RepoInfo, Repository,
};
use crate::state::{AppState, GitHubAction, Modal, PendingAction};

mod draw;

pub use draw::render;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    PullRequests,
    Repositories,
}

/// What the modal is doing with the selected pull request, if anything
/// beyond showing it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Browse,
    /// Writing what goes with a review or a comment.
    Compose { kind: Compose, text: String },
    /// Choosing how to merge.
    Merge(MergeForm),
    /// Asking before a pull request is closed.
    ConfirmClose,
    /// Writing a new pull request for the current branch.
    Create(CreateForm),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compose {
    Approve,
    RequestChanges,
    Comment,
}

impl Compose {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approve => "Approve",
            Self::RequestChanges => "Request changes on",
            Self::Comment => "Comment on",
        }
    }

    /// Approving can go without a word; the other two are nothing but words.
    fn needs_text(self) -> bool {
        !matches!(self, Self::Approve)
    }
}

/// The rows of the merge form, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeRow {
    Method,
    DeleteBranch,
    Auto,
}

impl MergeRow {
    pub const ALL: [Self; 3] = [Self::Method, Self::DeleteBranch, Self::Auto];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeForm {
    pub number: u64,
    pub method: MergeMethod,
    /// The methods the repository accepts; `method` is always one of them.
    pub methods: Vec<MergeMethod>,
    pub delete_branch: bool,
    pub auto: bool,
    pub row: MergeRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateField {
    Title,
    Base,
    Body,
    Draft,
}

impl CreateField {
    pub const ALL: [Self; 4] = [Self::Title, Self::Base, Self::Body, Self::Draft];

    fn next(self, forward: bool) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        }]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateForm {
    pub branch: String,
    pub title: String,
    pub base: String,
    pub body: String,
    pub draft: bool,
    pub field: CreateField,
}

/// A list on its way from GitHub.
type Pending<T> = Receiver<Result<T, String>>;

#[derive(Debug, Default)]
pub struct GitHub {
    pub tab: Tab,
    pub mode: Mode,
    pub filter: PrFilter,

    /// The repository the pull requests belong to, read once per checkout.
    pub repo: Option<RepoInfo>,
    pub prs: Vec<PullRequest>,
    pub pr_idx: usize,
    pub prs_error: Option<String>,
    pub prs_loading: Option<Pending<(Option<RepoInfo>, Vec<PullRequest>)>>,
    /// The checkout the pull requests were read from, so another checkout's
    /// list is never shown as this one's.
    pub prs_dir: Option<PathBuf>,
    /// How far the selected pull request's description is scrolled.
    pub detail_scroll: u16,

    pub repos: Vec<Repository>,
    pub repo_idx: usize,
    pub repos_error: Option<String>,
    pub repos_loading: Option<Pending<Vec<Repository>>>,
    /// Whose repositories are listed: one of the organizations in `owners`,
    /// or the signed-in user when `None`.
    pub owner: Option<String>,
    /// The user and their organizations, read with the first repository list.
    pub owners: Option<Owners>,
    pub owners_loading: Option<Pending<Owners>>,
    /// What the repository list is narrowed to, typed straight into the tab.
    pub query: String,

    /// Where a clone in flight is going, so lg can move there once it lands.
    pub clone_target: Option<PathBuf>,
}

impl GitHub {
    pub fn loading(&self) -> bool {
        self.prs_loading.is_some() || self.repos_loading.is_some() || self.owners_loading.is_some()
    }

    /// Everyone whose repositories can be listed, in the order ←/→ steps
    /// through them: the user first, then each organization.
    pub fn owner_choices(&self) -> Vec<Option<String>> {
        let orgs = self.owners.iter().flat_map(|owners| owners.orgs.iter());
        std::iter::once(None)
            .chain(orgs.cloned().map(Some))
            .collect()
    }
}

// ─── Opening and loading ─────────────────────────────────────────────────────

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
    state.github.tab = if state.repo_root.is_some() {
        Tab::PullRequests
    } else {
        Tab::Repositories
    };
    state.modal = Modal::GitHub;
    match state.github.tab {
        Tab::PullRequests => load_pull_requests(state),
        Tab::Repositories => load_repositories(state),
    }
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
    let need_info = state.github.repo.is_none();
    let (tx, rx) = std::sync::mpsc::channel();
    // Left to finish on its own if replaced: it only reads, and its answer is
    // dropped along with the channel.
    drop(crate::git::spawn_pinned(move || {
        let result = (|| {
            let info = if need_info {
                Some(crate::github::repo_info()?)
            } else {
                None
            };
            Ok::<_, anyhow::Error>((info, crate::github::pull_requests(filter)?))
        })();
        let _ = tx.send(result.map_err(|err| format!("{err:#}")));
    }));
    state.github.prs_loading = Some(rx);
    state.github.prs_dir = crate::git::active_repo();
    state.github.prs_error = None;
}

pub fn load_repositories(state: &mut AppState) {
    let owner = state.github.owner.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    drop(crate::git::spawn_pinned(move || {
        let repos = crate::github::repositories(owner.as_deref());
        let _ = tx.send(repos.map_err(|err| format!("{err:#}")));
    }));
    state.github.repos_loading = Some(rx);
    state.github.repos_error = None;
    if state.github.owners.is_none() {
        load_owners(state);
    }
}

/// Read which organizations the user belongs to, unless that is already
/// under way.
fn load_owners(state: &mut AppState) {
    if state.github.owners_loading.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    drop(crate::git::spawn_pinned(move || {
        let _ = tx.send(crate::github::owners().map_err(|err| format!("{err:#}")));
    }));
    state.github.owners_loading = Some(rx);
}

/// List the repositories of the next owner along — or the previous one.
fn step_owner(state: &mut AppState, forward: bool) {
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

/// A GitHub action has finished: what the list shows may no longer be true.
pub fn after_action(state: &mut AppState) {
    if state.modal == Modal::GitHub {
        load_pull_requests(state);
    }
}

/// Take in whatever the lists being read have sent back.
pub fn poll(state: &mut AppState) {
    let gh = &mut state.github;
    if let Some(result) = take_answer(&mut gh.prs_loading) {
        match result {
            Ok((info, prs)) => {
                if info.is_some() {
                    gh.repo = info;
                }
                gh.prs = prs;
                gh.pr_idx = gh.pr_idx.min(gh.prs.len().saturating_sub(1));
                gh.prs_error = None;
            }
            Err(err) => gh.prs_error = Some(err),
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
        Some(Ok(owners)) => state.github.owners = Some(owners),
        // Only the organizations are missing; the user's own list stands.
        Some(Err(err)) => {
            state.set_status(format!("could not read your organizations: {err}"), true)
        }
        None => {}
    }
    let visible = visible_repos(&state.github).len();
    state.github.repo_idx = state.github.repo_idx.min(visible.saturating_sub(1));
}

fn take_answer<T>(slot: &mut Option<Pending<T>>) -> Option<Result<T, String>> {
    let answer = match slot.as_ref()?.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => {
            Err("the request to GitHub ended without an answer".into())
        }
    };
    *slot = None;
    Some(answer)
}

// ─── What is selected ────────────────────────────────────────────────────────

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

/// Whether `pr` is the pull request for the branch checked out here.
pub fn is_current_branch(state: &AppState, pr: &PullRequest) -> bool {
    pr.head_owner.is_none() && state.branch.as_deref() == Some(pr.head.as_str())
}

// ─── Keys ────────────────────────────────────────────────────────────────────

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match state.github.mode {
        Mode::Browse => match state.github.tab {
            Tab::PullRequests => browse_pull_requests(state, key),
            Tab::Repositories => browse_repositories(state, key),
        },
        Mode::Compose { .. } => compose_key(state, key),
        Mode::Merge(_) => merge_key(state, key),
        Mode::ConfirmClose => confirm_close_key(state, key),
        Mode::Create(_) => create_key(state, key),
    }
    Ok(())
}

fn switch_tab(state: &mut AppState) {
    state.github.tab = match state.github.tab {
        Tab::PullRequests => Tab::Repositories,
        Tab::Repositories => Tab::PullRequests,
    };
    match state.github.tab {
        Tab::PullRequests if state.github.prs_loading.is_none() && state.github.prs.is_empty() => {
            load_pull_requests(state)
        }
        Tab::Repositories
            if state.github.repos_loading.is_none() && state.github.repos.is_empty() =>
        {
            load_repositories(state)
        }
        _ => {}
    }
}

fn select_pr(state: &mut AppState, idx: usize) {
    state.github.pr_idx = idx.min(state.github.prs.len().saturating_sub(1));
    state.github.detail_scroll = 0;
}

fn dispatch(state: &mut AppState, action: GitHubAction) {
    state.pending_action = Some(PendingAction::GitHub(action));
}

fn browse_pull_requests(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
            return;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            switch_tab(state);
            return;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            select_pr(state, state.github.pr_idx + 1);
            return;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            select_pr(state, state.github.pr_idx.saturating_sub(1));
            return;
        }
        KeyCode::Char('g') | KeyCode::Home => {
            select_pr(state, 0);
            return;
        }
        KeyCode::Char('G') | KeyCode::End => {
            select_pr(state, usize::MAX);
            return;
        }
        KeyCode::Char('d') if ctrl => {
            state.github.detail_scroll = state.github.detail_scroll.saturating_add(5);
            return;
        }
        KeyCode::Char('u') if ctrl => {
            state.github.detail_scroll = state.github.detail_scroll.saturating_sub(5);
            return;
        }
        KeyCode::Char('r') => {
            load_pull_requests(state);
            return;
        }
        KeyCode::Char('s') => {
            state.github.filter = state.github.filter.next();
            select_pr(state, 0);
            load_pull_requests(state);
            return;
        }
        KeyCode::Char('n') => {
            start_create(state);
            return;
        }
        KeyCode::Char('o') => {
            let url = selected_pr(state)
                .map(|pr| pr.url.clone())
                .or_else(|| state.github.repo.as_ref().map(|repo| repo.url.clone()))
                .filter(|url| !url.is_empty());
            match url {
                Some(url) => dispatch(state, GitHubAction::OpenInBrowser { url }),
                None => state.set_status("nothing to open yet", false),
            }
            return;
        }
        _ => {}
    }

    let Some(pr) = selected_pr(state).cloned() else {
        if matches!(key.code, KeyCode::Char(_) | KeyCode::Enter) {
            state.set_status("no pull request selected", false);
        }
        return;
    };
    let number = pr.number;
    // Reviewing, merging and drafting are for pull requests still open;
    // saying why is better than a gh error about it.
    let still_open = |state: &mut AppState| {
        if pr.is_open() {
            true
        } else {
            state.set_status(format!("#{number} is {}", pr.state.label()), false);
            false
        }
    };
    match key.code {
        KeyCode::Enter => {
            state.modal = Modal::None;
            dispatch(state, GitHubAction::Checkout { number });
        }
        KeyCode::Char('w') => match worktree_path_for(state, &pr) {
            Some(path) => {
                state.modal = Modal::None;
                dispatch(state, GitHubAction::CheckoutWorktree { number, path });
            }
            None => state.set_status("no main worktree to put a new one next to", true),
        },
        KeyCode::Char('a') if still_open(state) => compose(state, Compose::Approve),
        KeyCode::Char('x') if still_open(state) => compose(state, Compose::RequestChanges),
        KeyCode::Char('c') => compose(state, Compose::Comment),
        KeyCode::Char('m') if still_open(state) => start_merge(state, &pr),
        KeyCode::Char('D') if still_open(state) => dispatch(
            state,
            GitHubAction::SetDraft {
                number,
                draft: !pr.draft,
            },
        ),
        KeyCode::Char('X') => match pr.state {
            crate::github::PrState::Open => state.github.mode = Mode::ConfirmClose,
            crate::github::PrState::Closed => dispatch(state, GitHubAction::Reopen { number }),
            crate::github::PrState::Merged => {
                state.set_status(format!("#{number} is merged and cannot be reopened"), false)
            }
        },
        _ => {}
    }
}

/// Where a pull request checked out into a worktree goes: beside the main
/// worktree, named after its branch, the same as `n` would put it.
fn worktree_path_for(state: &AppState, pr: &PullRequest) -> Option<String> {
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

fn compose(state: &mut AppState, kind: Compose) {
    state.github.mode = Mode::Compose {
        kind,
        text: String::new(),
    };
}

fn start_merge(state: &mut AppState, pr: &PullRequest) {
    let (methods, method) = match &state.github.repo {
        Some(repo) if !repo.merge_methods.is_empty() => {
            (repo.merge_methods.clone(), repo.preferred_merge)
        }
        _ => (MergeMethod::ALL.to_vec(), MergeMethod::Merge),
    };
    state.github.mode = Mode::Merge(MergeForm {
        number: pr.number,
        method,
        methods,
        delete_branch: true,
        auto: false,
        row: MergeRow::Method,
    });
}

/// Open the new pull request form for the branch checked out here, filled in
/// from what the branch carries — or from the PR text the review wrote, when
/// there is one.
fn start_create(state: &mut AppState) {
    let Some(branch) = state.branch.clone() else {
        state.set_status("check a branch out to open a pull request from it", false);
        return;
    };
    if let Some(idx) = state
        .github
        .prs
        .iter()
        .position(|pr| pr.is_open() && is_current_branch(state, pr))
    {
        let number = state.github.prs[idx].number;
        select_pr(state, idx);
        state.set_status(format!("#{number} is already open for {branch}"), false);
        return;
    }
    let base = state
        .github
        .repo
        .as_ref()
        .map(|repo| repo.default_branch.clone())
        .unwrap_or_else(|| "main".to_string());
    if branch == base {
        state.set_status(
            format!("{branch} is the base branch; open pull requests from a feature branch"),
            false,
        );
        return;
    }
    let remote = crate::preferences::remote();
    let commits = crate::git::commit_messages_since(&format!("{remote}/{base}"))
        .or_else(|_| crate::git::commit_messages_since(&base))
        .unwrap_or_default();
    let (title, mut body) = crate::github::fill_from_commits(&branch, &commits);
    if let Some(text) = state
        .review_assists
        .get(crate::git::REVIEW_PR_TEXT_NODE_ID)
        .filter(|text| !text.trim().is_empty())
    {
        body = text.trim().to_string();
    }
    state.github.mode = Mode::Create(CreateForm {
        branch,
        title,
        base,
        body,
        draft: false,
        field: CreateField::Title,
    });
}

fn compose_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Mode::Compose { kind, text } = &mut state.github.mode else {
        return;
    };
    let kind = *kind;
    match key.code {
        KeyCode::Esc => state.github.mode = Mode::Browse,
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Char(c) if !ctrl => text.push(c),
        KeyCode::Enter => {
            let body = text.trim().to_string();
            if kind.needs_text() && body.is_empty() {
                state.set_status("write something first; Esc cancels", false);
                return;
            }
            let Some(number) = selected_pr(state).map(|pr| pr.number) else {
                state.github.mode = Mode::Browse;
                return;
            };
            state.github.mode = Mode::Browse;
            let action = match kind {
                Compose::Approve => GitHubAction::Review {
                    number,
                    verdict: crate::github::Verdict::Approve,
                    body,
                },
                Compose::RequestChanges => GitHubAction::Review {
                    number,
                    verdict: crate::github::Verdict::RequestChanges,
                    body,
                },
                Compose::Comment => GitHubAction::Comment { number, body },
            };
            dispatch(state, action);
        }
        _ => {}
    }
}

fn merge_key(state: &mut AppState, key: KeyEvent) {
    let Mode::Merge(form) = &mut state.github.mode else {
        return;
    };
    let row_idx = MergeRow::ALL
        .iter()
        .position(|row| *row == form.row)
        .unwrap_or(0);
    let rows = MergeRow::ALL.len();
    match key.code {
        KeyCode::Esc => state.github.mode = Mode::Browse,
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
            form.row = MergeRow::ALL[(row_idx + 1) % rows];
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            form.row = MergeRow::ALL[(row_idx + rows - 1) % rows];
        }
        KeyCode::Char(' ' | 'h' | 'l') | KeyCode::Left | KeyCode::Right => {
            let forward = !matches!(key.code, KeyCode::Char('h') | KeyCode::Left);
            match form.row {
                MergeRow::Method => {
                    let len = form.methods.len().max(1);
                    let idx = form
                        .methods
                        .iter()
                        .position(|method| *method == form.method)
                        .unwrap_or(0);
                    let next = if forward {
                        (idx + 1) % len
                    } else {
                        (idx + len - 1) % len
                    };
                    if let Some(method) = form.methods.get(next) {
                        form.method = *method;
                    }
                }
                MergeRow::DeleteBranch => form.delete_branch = !form.delete_branch,
                MergeRow::Auto => form.auto = !form.auto,
            }
        }
        KeyCode::Enter => {
            let action = GitHubAction::Merge {
                number: form.number,
                method: form.method,
                delete_branch: form.delete_branch,
                auto: form.auto,
            };
            state.github.mode = Mode::Browse;
            dispatch(state, action);
        }
        _ => {}
    }
}

fn confirm_close_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y' | 'Y') => {
            state.github.mode = Mode::Browse;
            if let Some(number) = selected_pr(state).map(|pr| pr.number) {
                dispatch(state, GitHubAction::Close { number });
            }
        }
        KeyCode::Char('n' | 'N') | KeyCode::Esc => state.github.mode = Mode::Browse,
        _ => {}
    }
}

fn create_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Mode::Create(form) = &mut state.github.mode else {
        return;
    };
    match key.code {
        KeyCode::Esc => state.github.mode = Mode::Browse,
        KeyCode::Char('s') if ctrl => submit_create(state),
        KeyCode::Tab | KeyCode::Down => form.field = form.field.next(true),
        KeyCode::BackTab | KeyCode::Up => form.field = form.field.next(false),
        KeyCode::Enter => match form.field {
            CreateField::Body => form.body.push('\n'),
            CreateField::Draft => form.draft = !form.draft,
            CreateField::Title | CreateField::Base => form.field = form.field.next(true),
        },
        KeyCode::Char(' ') if form.field == CreateField::Draft => form.draft = !form.draft,
        KeyCode::Char('u') if ctrl => {
            if let Some(text) = create_text(form) {
                text.clear();
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if let Some(text) = create_text(form) {
                text.push(c);
            }
        }
        KeyCode::Backspace => {
            if let Some(text) = create_text(form) {
                text.pop();
            }
        }
        _ => {}
    }
}

/// The text the focused field of the new pull request form edits.
fn create_text(form: &mut CreateForm) -> Option<&mut String> {
    match form.field {
        CreateField::Title => Some(&mut form.title),
        CreateField::Base => Some(&mut form.base),
        CreateField::Body => Some(&mut form.body),
        CreateField::Draft => None,
    }
}

fn submit_create(state: &mut AppState) {
    let Mode::Create(form) = &mut state.github.mode else {
        return;
    };
    if form.title.trim().is_empty() {
        form.field = CreateField::Title;
        state.set_status("a pull request needs a title", true);
        return;
    }
    if form.base.trim().is_empty() {
        form.field = CreateField::Base;
        state.set_status("a pull request needs a base branch", true);
        return;
    }
    let pr = NewPullRequest {
        branch: form.branch.clone(),
        base: form.base.trim().to_string(),
        title: form.title.trim().to_string(),
        body: form.body.trim_end().to_string(),
        draft: form.draft,
    };
    state.github.mode = Mode::Browse;
    dispatch(state, GitHubAction::Create(pr));
}

fn browse_repositories(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let visible = visible_repos(&state.github).len();
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Tab | KeyCode::BackTab => switch_tab(state),
        KeyCode::Down => {
            state.github.repo_idx = (state.github.repo_idx + 1).min(visible.saturating_sub(1))
        }
        KeyCode::Up => state.github.repo_idx = state.github.repo_idx.saturating_sub(1),
        KeyCode::Right => step_owner(state, true),
        KeyCode::Left => step_owner(state, false),
        KeyCode::Char('r') if ctrl => {
            load_owners(state);
            load_repositories(state);
        }
        KeyCode::Char('u') if ctrl => {
            state.github.query.clear();
            state.github.repo_idx = 0;
        }
        KeyCode::Backspace => {
            state.github.query.pop();
            state.github.repo_idx = 0;
        }
        KeyCode::Char(c) if !ctrl => {
            state.github.query.push(c);
            state.github.repo_idx = 0;
        }
        KeyCode::Enter => clone_selected(state),
        _ => {}
    }
}

fn clone_selected(state: &mut AppState) {
    let Some(repo) = clone_request(state) else {
        state.set_status("pick a repository, or type owner/name", false);
        return;
    };
    let Some(dir) = clone_destination(state) else {
        state.set_status(format!("no folder to clone {repo} into"), true);
        return;
    };
    if dir.exists() {
        // Already here: going to it is what cloning it again would have been for.
        if crate::git::repo_root_at(&dir).is_some() {
            state.modal = Modal::None;
            state.pending_action = Some(PendingAction::SwitchRepository {
                target: crate::state::RepoTarget::Path(dir),
            });
        } else {
            state.set_status(
                format!("{} exists and is not a repository", dir.display()),
                true,
            );
        }
        return;
    }
    state.modal = Modal::None;
    dispatch(
        state,
        GitHubAction::Clone {
            repo,
            dir: dir.to_string_lossy().into_owned(),
        },
    );
}

/// Text pasted while the modal takes typing. Returns whether it was taken.
pub fn handle_paste(state: &mut AppState, text: &str) -> bool {
    if state.modal != Modal::GitHub {
        return false;
    }
    let one_line = || text.replace(['\r', '\n'], " ");
    match &mut state.github.mode {
        Mode::Compose { text: typed, .. } => typed.push_str(&one_line()),
        Mode::Create(form) => match form.field {
            CreateField::Body => form.body.push_str(&text.replace('\r', "")),
            CreateField::Title => form.title.push_str(&one_line()),
            CreateField::Base => form.base.push_str(one_line().trim()),
            CreateField::Draft => return false,
        },
        Mode::Browse if state.github.tab == Tab::Repositories => {
            state.github.query.push_str(one_line().trim());
            state.github.repo_idx = 0;
        }
        _ => return false,
    }
    true
}

/// The wheel moves through whichever list is on screen.
pub fn handle_mouse(state: &mut AppState, m: &MouseEvent) {
    if state.github.mode != Mode::Browse {
        return;
    }
    let down = match m.kind {
        MouseEventKind::ScrollDown => true,
        MouseEventKind::ScrollUp => false,
        _ => return,
    };
    match state.github.tab {
        Tab::PullRequests if down => select_pr(state, state.github.pr_idx + 1),
        Tab::PullRequests => select_pr(state, state.github.pr_idx.saturating_sub(1)),
        Tab::Repositories => {
            let last = visible_repos(&state.github).len().saturating_sub(1);
            state.github.repo_idx = if down {
                (state.github.repo_idx + 1).min(last)
            } else {
                state.github.repo_idx.saturating_sub(1)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Checks, Mergeable, PrState, ReviewDecision, Verdict};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(state: &mut AppState, text: &str) {
        for c in text.chars() {
            handle_key(state, key(KeyCode::Char(c))).unwrap();
        }
    }

    fn pr(number: u64, head: &str) -> PullRequest {
        PullRequest {
            number,
            title: format!("PR {number}"),
            author: "alice".into(),
            head: head.into(),
            head_owner: None,
            base: "main".into(),
            state: PrState::Open,
            draft: false,
            review: ReviewDecision::ReviewRequired,
            reviews: Vec::new(),
            checks: Checks::default(),
            mergeable: Mergeable::Yes,
            url: format!("https://github.com/acme/app/pull/{number}"),
            body: String::new(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            updated_at: String::new(),
        }
    }

    fn repo(name: &str, description: &str) -> Repository {
        Repository {
            name: name.into(),
            description: description.into(),
            private: false,
            fork: false,
            archived: false,
        }
    }

    /// The modal open on the pull requests of a checkout, as if the list had
    /// just come back from GitHub.
    fn with_prs(prs: Vec<PullRequest>) -> AppState {
        let mut state = AppState::new();
        state.repo_root = Some("/work/app".into());
        state.branch = Some("feat/login".into());
        state.modal = Modal::GitHub;
        state.github.prs = prs;
        state.github.repo = Some(RepoInfo {
            name: "acme/app".into(),
            url: "https://github.com/acme/app".into(),
            default_branch: "main".into(),
            merge_methods: vec![MergeMethod::Squash, MergeMethod::Rebase],
            preferred_merge: MergeMethod::Squash,
        });
        state
    }

    fn github_action(state: &AppState) -> Option<GitHubAction> {
        match &state.pending_action {
            Some(PendingAction::GitHub(action)) => Some(action.clone()),
            _ => None,
        }
    }

    #[test]
    fn enter_checks_the_selected_pull_request_out_here() {
        let mut state = with_prs(vec![pr(1, "a"), pr(2, "b")]);
        handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Checkout { number: 2 })
        );
        assert_eq!(
            state.modal,
            Modal::None,
            "the checkout is what to look at next"
        );
    }

    #[test]
    fn a_pull_request_can_be_checked_out_into_its_own_worktree() {
        let mut state = with_prs(vec![pr(9, "feat/x")]);
        handle_key(&mut state, key(KeyCode::Char('w'))).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::CheckoutWorktree {
                number: 9,
                path: "/work/app.worktrees/feat-x".into(),
            })
        );
    }

    #[test]
    fn approving_goes_out_with_the_words_typed_and_keeps_the_list_open() {
        let mut state = with_prs(vec![pr(5, "a")]);
        handle_key(&mut state, key(KeyCode::Char('a'))).unwrap();
        assert!(
            github_action(&state).is_none(),
            "nothing is sent before Enter"
        );
        type_text(&mut state, "lgtm");
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Review {
                number: 5,
                verdict: Verdict::Approve,
                body: "lgtm".into(),
            })
        );
        assert_eq!(state.modal, Modal::GitHub);
        assert_eq!(state.github.mode, Mode::Browse);
    }

    #[test]
    fn an_approval_needs_no_words() {
        let mut state = with_prs(vec![pr(5, "a")]);
        handle_key(&mut state, key(KeyCode::Char('a'))).unwrap();
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert!(matches!(
            github_action(&state),
            Some(GitHubAction::Review {
                verdict: Verdict::Approve,
                ..
            })
        ));
    }

    #[test]
    fn requesting_changes_waits_until_it_says_what_to_change() {
        let mut state = with_prs(vec![pr(5, "a")]);
        handle_key(&mut state, key(KeyCode::Char('x'))).unwrap();
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();
        assert!(github_action(&state).is_none());

        type_text(&mut state, "needs tests");
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();
        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Review {
                number: 5,
                verdict: Verdict::RequestChanges,
                body: "needs tests".into(),
            })
        );
    }

    #[test]
    fn escape_abandons_a_comment_without_sending_it() {
        let mut state = with_prs(vec![pr(5, "a")]);
        handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();
        type_text(&mut state, "half a thought");
        handle_key(&mut state, key(KeyCode::Esc)).unwrap();

        assert!(github_action(&state).is_none());
        assert_eq!(state.github.mode, Mode::Browse);
        assert_eq!(
            state.modal,
            Modal::GitHub,
            "Esc backs out of the comment only"
        );
    }

    #[test]
    fn merging_offers_only_what_the_repository_allows() {
        let mut state = with_prs(vec![pr(3, "a")]);
        handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();
        // Step the method past the end of the list and back round to the start.
        handle_key(&mut state, key(KeyCode::Right)).unwrap();
        handle_key(&mut state, key(KeyCode::Right)).unwrap();
        // Keep the branch, and merge.
        handle_key(&mut state, key(KeyCode::Down)).unwrap();
        handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Merge {
                number: 3,
                method: MergeMethod::Squash,
                delete_branch: false,
                auto: false,
            })
        );
    }

    #[test]
    fn a_merged_pull_request_is_not_offered_for_merging_again() {
        let mut merged = pr(3, "a");
        merged.state = PrState::Merged;
        let mut state = with_prs(vec![merged]);
        handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();

        assert_eq!(state.github.mode, Mode::Browse);
        assert!(github_action(&state).is_none());
        assert!(state.status.is_some(), "the user is told why");
    }

    #[test]
    fn closing_asks_first_and_a_closed_one_reopens() {
        let mut state = with_prs(vec![pr(4, "a")]);
        handle_key(&mut state, key(KeyCode::Char('X'))).unwrap();
        assert!(github_action(&state).is_none());
        handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();
        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Close { number: 4 })
        );

        let mut closed = pr(4, "a");
        closed.state = PrState::Closed;
        let mut state = with_prs(vec![closed]);
        handle_key(&mut state, key(KeyCode::Char('X'))).unwrap();
        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Reopen { number: 4 })
        );
    }

    #[test]
    fn a_new_pull_request_goes_from_the_current_branch_to_the_default_one() {
        let mut state = with_prs(Vec::new());
        handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();
        let Mode::Create(form) = &mut state.github.mode else {
            panic!("the form opens");
        };
        form.title.clear();
        type_text(&mut state, "Add login");
        // Title, Base, Body.
        handle_key(&mut state, key(KeyCode::Tab)).unwrap();
        handle_key(&mut state, key(KeyCode::Tab)).unwrap();
        if let Mode::Create(form) = &mut state.github.mode {
            form.body.clear();
        }
        type_text(&mut state, "first");
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();
        type_text(&mut state, "second");
        handle_key(&mut state, key(KeyCode::Tab)).unwrap();
        handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
        handle_key(&mut state, ctrl('s')).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Create(NewPullRequest {
                branch: "feat/login".into(),
                base: "main".into(),
                title: "Add login".into(),
                body: "first\nsecond".into(),
                draft: true,
            }))
        );
    }

    #[test]
    fn a_branch_that_already_has_a_pull_request_goes_to_it_instead() {
        let mut state = with_prs(vec![pr(1, "other"), pr(8, "feat/login")]);
        handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();

        assert_eq!(state.github.mode, Mode::Browse);
        assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(8));
    }

    #[test]
    fn the_review_pr_text_becomes_the_body_of_a_new_pull_request() {
        let mut state = with_prs(Vec::new());
        state.review_assists.insert(
            crate::git::REVIEW_PR_TEXT_NODE_ID.to_string(),
            "## Summary\n- adds login".into(),
        );
        handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();

        let Mode::Create(form) = &state.github.mode else {
            panic!("the form opens");
        };
        assert_eq!(form.body, "## Summary\n- adds login");
    }

    #[test]
    fn no_pull_request_is_opened_from_the_base_branch() {
        let mut state = with_prs(Vec::new());
        state.branch = Some("main".into());
        handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();

        assert_eq!(state.github.mode, Mode::Browse);
    }

    #[test]
    fn typing_narrows_the_repositories_and_enter_clones_into_the_workspace() {
        let mut state = AppState::new();
        state.workspace_root = Some("/nonexistent-lg-workspace".into());
        state.modal = Modal::GitHub;
        state.github.tab = Tab::Repositories;
        state.github.repos = vec![
            repo("me/lg", "a git tui"),
            repo("me/notes", "plain text"),
            repo("me/blog", ""),
        ];

        type_text(&mut state, "text");
        assert_eq!(
            visible_repos(&state.github)
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["me/notes"],
            "descriptions are searched too"
        );
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Clone {
                repo: "me/notes".into(),
                dir: "/nonexistent-lg-workspace/notes".into(),
            })
        );
    }

    #[test]
    fn a_typed_owner_and_name_can_be_cloned_without_being_listed() {
        let mut state = AppState::new();
        state.repo_root = Some("/nonexistent-lg-parent/current".into());
        state.modal = Modal::GitHub;
        state.github.tab = Tab::Repositories;

        type_text(&mut state, "rust-lang/rust");
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Clone {
                repo: "rust-lang/rust".into(),
                dir: "/nonexistent-lg-parent/rust".into(),
            }),
            "a clone goes beside the checkout, never inside it"
        );
    }

    /// The repositories tab listing the user's own, with the organizations
    /// they belong to already read.
    fn with_owners(orgs: &[&str]) -> AppState {
        let mut state = AppState::new();
        state.workspace_root = Some("/nonexistent-lg-workspace".into());
        state.modal = Modal::GitHub;
        state.github.tab = Tab::Repositories;
        state.github.owners = Some(Owners {
            login: "me".into(),
            orgs: orgs.iter().map(|org| org.to_string()).collect(),
        });
        state.github.repos = vec![repo("me/lg", "")];
        state
    }

    #[test]
    fn the_arrows_step_through_the_users_organizations_and_back_to_the_user() {
        let mut state = with_owners(&["acme", "zeta"]);

        handle_key(&mut state, key(KeyCode::Right)).unwrap();
        assert_eq!(state.github.owner.as_deref(), Some("acme"));
        handle_key(&mut state, key(KeyCode::Right)).unwrap();
        assert_eq!(state.github.owner.as_deref(), Some("zeta"));
        handle_key(&mut state, key(KeyCode::Right)).unwrap();
        assert_eq!(
            state.github.owner, None,
            "past the last, the user's own again"
        );
        handle_key(&mut state, key(KeyCode::Left)).unwrap();
        assert_eq!(state.github.owner.as_deref(), Some("zeta"));
    }

    #[test]
    fn another_owners_list_never_clones_from_the_last_one() {
        let mut state = with_owners(&["acme"]);

        handle_key(&mut state, key(KeyCode::Right)).unwrap();
        assert!(
            visible_repos(&state.github).is_empty(),
            "me/lg is not acme's to offer"
        );

        state.github.repos = vec![repo("acme/api", "")];
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();
        assert_eq!(
            github_action(&state),
            Some(GitHubAction::Clone {
                repo: "acme/api".into(),
                dir: "/nonexistent-lg-workspace/api".into(),
            })
        );
    }

    #[test]
    fn without_organizations_the_arrows_say_why_nothing_changed() {
        let mut state = with_owners(&[]);

        handle_key(&mut state, key(KeyCode::Right)).unwrap();

        assert_eq!(state.github.owner, None);
        assert_eq!(visible_repos(&state.github).len(), 1, "the list is kept");
        assert!(state.status.is_some());
    }

    #[test]
    fn organizations_that_could_not_be_read_leave_the_users_list_standing() {
        let mut state = with_owners(&[]);
        state.github.owners = None;
        let (tx, rx) = std::sync::mpsc::channel();
        state.github.owners_loading = Some(rx);
        tx.send(Err("missing read:org scope".into())).unwrap();

        poll(&mut state);

        assert!(!state.github.loading());
        assert!(state.status.as_ref().is_some_and(|status| status.is_error));
        assert_eq!(
            visible_repos(&state.github)
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["me/lg"]
        );
    }

    #[test]
    fn a_repository_already_cloned_is_opened_rather_than_cloned_again() {
        let workspace = tempfile::tempdir().unwrap();
        let existing = workspace.path().join("lg");
        std::fs::create_dir(&existing).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&existing)
            .status()
            .unwrap();
        assert!(init.success());

        let mut state = AppState::new();
        state.workspace_root = Some(workspace.path().to_string_lossy().into_owned());
        state.modal = Modal::GitHub;
        state.github.tab = Tab::Repositories;
        state.github.repos = vec![repo("me/lg", "")];
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(
            state.pending_action,
            Some(PendingAction::SwitchRepository {
                target: crate::state::RepoTarget::Path(existing),
            })
        );
    }

    #[test]
    fn a_pasted_body_keeps_its_lines_and_a_pasted_title_does_not() {
        let mut state = with_prs(Vec::new());
        handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();
        if let Mode::Create(form) = &mut state.github.mode {
            form.title.clear();
            form.body.clear();
        }
        assert!(handle_paste(&mut state, "Fix\nlogin"));
        handle_key(&mut state, key(KeyCode::Tab)).unwrap();
        handle_key(&mut state, key(KeyCode::Tab)).unwrap();
        assert!(handle_paste(&mut state, "line one\nline two"));

        let Mode::Create(form) = &state.github.mode else {
            panic!("the form is still open");
        };
        assert_eq!(form.title, "Fix login");
        assert_eq!(form.body, "line one\nline two");
    }

    #[test]
    fn a_finished_list_replaces_what_was_shown() {
        let mut state = with_prs(vec![pr(1, "a"), pr(2, "b"), pr(3, "c")]);
        state.github.pr_idx = 2;
        let (tx, rx) = std::sync::mpsc::channel();
        state.github.prs_loading = Some(rx);
        tx.send(Ok((None, vec![pr(10, "x")]))).unwrap();

        poll(&mut state);

        assert!(!state.github.loading());
        assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(10));
        assert!(state.github.repo.is_some(), "repository details are kept");
    }

    #[test]
    fn a_failed_list_says_why() {
        let mut state = with_prs(Vec::new());
        let (tx, rx) = std::sync::mpsc::channel();
        state.github.prs_loading = Some(rx);
        tx.send(Err("gh auth login required".into())).unwrap();

        poll(&mut state);

        assert_eq!(
            state.github.prs_error.as_deref(),
            Some("gh auth login required")
        );
    }
}
