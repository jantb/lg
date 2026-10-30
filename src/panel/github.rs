//! GitHub from inside lg: what is waiting on the user across the workspace,
//! the pull requests of the checkout on screen — check one out, review it,
//! merge it, open a new one — and the repositories of the user or one of their
//! organizations, to clone one.
//!
//! Everything goes through `gh` (see [`crate::github`]). Lists are read on a
//! thread of their own and picked up by [`poll`]; anything that changes
//! something is handed to the app as a [`PendingAction::GitHub`], so it runs
//! and reports like every other operation.

use std::path::PathBuf;
use work::Pending;

use crate::github::{MergeMethod, Owners, PrFilter, PullRequest, RepoInfo, Repository};
use crate::panel::text_input::TextInput;

mod browse;
mod draw;
mod forms;
pub mod inbox;
mod load;
mod lookup;
mod mouse;
mod work;

pub use draw::render;

pub use browse::{handle_key, handle_paste};
pub use load::{after_action, load_pull_requests, load_repositories, open, open_create, poll};
pub use lookup::{
    clone_destination, clone_parent, clone_request, is_current_branch, is_mine, selected_pr,
    visible_repos,
};
pub use mouse::handle_mouse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// What is waiting on the user, in every repository of the workspace.
    Inbox,
    #[default]
    PullRequests,
    Repositories,
}

impl Tab {
    const ALL: [Self; 3] = [Self::Inbox, Self::PullRequests, Self::Repositories];

    fn next(self, forward: bool) -> Self {
        let idx = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        }]
    }
}

/// What the modal is doing with the selected pull request, if anything
/// beyond showing it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Browse,
    /// Writing what goes with a review or a comment.
    Compose { kind: Compose, text: TextInput },
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
    pub title: TextInput,
    pub base: TextInput,
    pub body: TextInput,
    pub draft: bool,
    pub field: CreateField,
}

#[derive(Debug, Default)]
pub struct GitHub {
    pub tab: Tab,
    pub mode: Mode,
    pub filter: PrFilter,
    /// Whether draft pull requests are left out of the list.
    pub hide_drafts: bool,

    pub inbox: inbox::Inbox,

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
    pub query: TextInput,

    /// Where a clone in flight is going, so lg can move there once it lands.
    pub clone_target: Option<PathBuf>,
    /// The new pull request form opens once the list arrives, since it needs
    /// to know whether the branch already has one.
    pub create_on_load: bool,
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

// ─── What is selected ────────────────────────────────────────────────────────

// ─── Keys ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
