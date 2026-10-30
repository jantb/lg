//! GitHub, through the `gh` command-line tool.
//!
//! lg does not speak the GitHub API itself. `gh` already holds the user's
//! login, knows which remote of a checkout is the GitHub one, and sets up the
//! fork's remote when a pull request from a fork is checked out, so every call
//! here is a `gh` invocation against the checkout lg is pointed at, read back
//! as JSON where there is anything to read.

mod actions;
mod cloning;
mod gh;
mod inbox;
mod parse;
mod query;
mod review;

pub use inbox::{Inbox, InboxItem, Reason, Section, Subject, mark_all_done, mark_done, read_inbox};

pub use actions::{
    NewPullRequest, checkout, checkout_in_worktree, close, create, fill_from_commits, merge,
    reopen, set_draft,
};
pub use cloning::{clone, clone_dir, open_in_browser};
pub use parse::{parse_owners, parse_pull_requests, parse_repo_info, parse_repositories};
pub use query::{owners, pr_diff, pr_head, pull_request, pull_requests, repo_info, repositories};
pub use review::{LineComment, ReviewEvent, comment, review, submit_review};

/// Which pull requests the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrFilter {
    #[default]
    Open,
    /// Open pull requests whose review is asked of the user by name, not
    /// merely of a team they are on.
    ReviewRequested,
    /// Open pull requests whose review is asked of a team the user is on, or
    /// of the user.
    TeamReviewRequested,
    /// Open pull requests by anyone but the user: everything they could
    /// review, asked or not.
    ToReview,
    /// The user's own that are still open.
    Mine,
}

impl PrFilter {
    pub const ALL: [Self; 5] = [
        Self::Open,
        Self::ReviewRequested,
        Self::TeamReviewRequested,
        Self::ToReview,
        Self::Mine,
    ];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ReviewRequested => "for me",
            Self::TeamReviewRequested => "my teams",
            Self::ToReview => "to review",
            Self::Mine => "mine",
        }
    }

    /// Merged and closed pull requests are never asked for: every list is of
    /// the open ones.
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Mine => &["--state", "open", "--author", "@me"],
            _ => &["--state", "open"],
        }
    }

    /// What the filter adds to a GitHub search, if it needs one.
    fn search(self) -> Option<&'static str> {
        match self {
            Self::ReviewRequested => Some("user-review-requested:@me"),
            Self::TeamReviewRequested => Some("review-requested:@me"),
            Self::ToReview => Some("-author:@me"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

/// Where the reviews of a pull request have left it, as GitHub sums them up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    /// The repository asks for no reviews.
    None,
}

impl ReviewDecision {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Approved => Some("approved"),
            Self::ChangesRequested => Some("changes requested"),
            Self::ReviewRequired => Some("review required"),
            Self::None => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Yes,
    Conflicting,
    /// GitHub has not worked it out yet; it does so lazily.
    Unknown,
}

/// The checks on a pull request's head commit, counted by outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks {
    pub passed: usize,
    pub pending: usize,
    /// Names of the checks that failed, since those are the ones to go and read.
    pub failed: Vec<String>,
}

impl Checks {
    pub fn is_empty(&self) -> bool {
        self.passed == 0 && self.pending == 0 && self.failed.is_empty()
    }
}

/// One reviewer's latest word on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, … as GitHub spells it.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    /// The branch the changes are on.
    pub head: String,
    /// Who owns the head branch's repository, for a pull request from a fork.
    pub head_owner: Option<String>,
    /// The branch they are to be merged into.
    pub base: String,
    pub state: PrState,
    pub draft: bool,
    pub review: ReviewDecision,
    pub reviews: Vec<Review>,
    pub checks: Checks,
    pub mergeable: Mergeable,
    pub url: String,
    pub body: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub updated_at: String,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == PrState::Open
    }
}

/// A repository offered for cloning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// `owner/name`.
    pub name: String,
    pub description: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [Self; 3] = [Self::Merge, Self::Squash, Self::Rebase];

    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "merge commit",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }

    fn from_github(name: &str) -> Option<Self> {
        match name {
            "MERGE" => Some(Self::Merge),
            "SQUASH" => Some(Self::Squash),
            "REBASE" => Some(Self::Rebase),
            _ => None,
        }
    }
}

/// Whose repositories there are to list: the signed-in user's own, and those
/// of each organization they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Owners {
    pub login: String,
    /// Sorted by name, so stepping through them goes in the order shown.
    pub orgs: Vec<String>,
}

/// What the repository a checkout belongs to allows and prefers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// `owner/name`.
    pub name: String,
    pub url: String,
    pub default_branch: String,
    /// The merge methods the repository accepts, in [`MergeMethod::ALL`] order.
    pub merge_methods: Vec<MergeMethod>,
    /// The method the user last merged with here, as GitHub remembers it.
    pub preferred_merge: MergeMethod,
}

/// A review that settles something, as opposed to a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    RequestChanges,
}

impl Verdict {
    fn flag(self) -> &'static str {
        match self {
            Self::Approve => "--approve",
            Self::RequestChanges => "--request-changes",
        }
    }
}

// ─── Reading ─────────────────────────────────────────────────────────────────

// ─── Acting ──────────────────────────────────────────────────────────────────

// ─── Pure helpers ────────────────────────────────────────────────────────────

// ─── Parsing ─────────────────────────────────────────────────────────────────

// ─── Running gh ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
