use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use crate::git::{
    AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo, ReleaseBranches,
    RemoteBranch, Worktree,
};

use super::DiffSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStyleSeverity {
    Ok,
    Warn,
    Fail,
}

impl ReviewStyleSeverity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewStyleFinding {
    pub severity: ReviewStyleSeverity,
    pub line: Option<usize>,
    pub reason: String,
}

/// A background job: the channel its worker reports on, the worker's handle, and
/// the spinner frame it is drawn at. Lets the drain loop treat every job the
/// same way, whatever else the job carries.
pub trait BackgroundJob {
    type Msg;

    fn rx(&self) -> &Receiver<Self::Msg>;
    fn handle_mut(&mut self) -> &mut Option<JoinHandle<()>>;
    fn spinner_mut(&mut self) -> &mut usize;
}

macro_rules! impl_background_job {
    ($($job:ident => $msg:ty),+ $(,)?) => { $(
        impl BackgroundJob for $job {
            type Msg = $msg;

            fn rx(&self) -> &Receiver<Self::Msg> {
                &self.rx
            }

            fn handle_mut(&mut self) -> &mut Option<JoinHandle<()>> {
                &mut self.handle
            }

            fn spinner_mut(&mut self) -> &mut usize {
                &mut self.spinner
            }
        }
    )+ };
}

impl_background_job! {
    Generation => GenMsg,
    PushJob => PushMsg,
    CheckoutJob => CheckoutMsg,
    OperationJob => OperationMsg,
    FetchJob => FetchMsg,
    RefreshJob => RefreshMsg,
    ReleaseStatusJob => ReleaseStatusMsg,
    SettingsSuggestJob => SettingsSuggestMsg,
    CommitLogJob => CommitLogMsg,
    DiffJob => DiffMsg,
    ReviewJob => ReviewMsg,
    ReviewAssistJob => GenMsg,
    ReviewFlagJob => ReviewFlagMsg,
    ReviewChatJob => GenMsg,
    WorkflowJob => WorkflowMsg,
}

#[derive(Debug)]
pub enum GenMsg {
    Thinking(String),
    Output(String),
    /// What has been sent as output turned out to be reasoning after all — a
    /// closing think tag arrived with no opening one. Consumers drop whatever
    /// they have accumulated and start over.
    Reset,
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct Generation {
    pub rx: Receiver<GenMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub output: String,
    pub spinner: usize,
}

pub const SPINNER_FRAMES: &[&str] = &[
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

#[derive(Debug)]
pub enum PushMsg {
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct PushJob {
    pub rx: Receiver<PushMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub branch: String,
    pub remote: String,
}

#[derive(Debug)]
pub enum CheckoutMsg {
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct CheckoutJob {
    pub rx: Receiver<CheckoutMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub branch: String,
}

#[derive(Debug)]
pub enum OperationMsg {
    /// What the operation is doing now. A multi-step one reports several of
    /// these before the message that ends it.
    Progress(String),
    Done(String),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Commit,
    StageAllAndCommit,
    MergeUpstream,
    Index,
    FileSystem,
    /// Moves what is checked out: pull, checkout, upstream changes. Named for
    /// the working tree it rewrites, so `worktree` stays free for git's own
    /// linked worktrees.
    WorkingTree,
}

#[derive(Debug)]
pub struct OperationJob {
    pub rx: Receiver<OperationMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub label: &'static str,
    pub kind: OperationKind,
    /// The step last reported, for operations that say more than their label.
    pub step: Option<String>,
}

#[derive(Debug)]
pub enum FetchMsg {
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct FetchJob {
    pub rx: Receiver<FetchMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
}

#[derive(Debug)]
pub enum RefreshMsg {
    Done(Box<RefreshSnapshot>),
}

#[derive(Debug)]
pub struct RefreshSnapshot {
    pub repo_root: Option<String>,
    pub workspace_root: Option<String>,
    pub files: Option<Vec<FileEntry>>,
    pub branches: Option<Vec<Branch>>,
    pub remote_branches: Option<Vec<RemoteBranch>>,
    pub nested_repositories: Option<Vec<NestedRepo>>,
    pub worktrees: Option<Vec<Worktree>>,
    pub release_branches: ReleaseBranches,
    pub commits: Option<Vec<Commit>>,
    pub unpushed_shas: Option<HashSet<String>>,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub ahead_behind: Option<(u32, u32)>,
    pub errors: Vec<String>,
}

#[derive(Debug)]
pub enum ReleaseStatusMsg {
    Done {
        branch: String,
        status: BranchReleaseStatus,
    },
    Error {
        branch: String,
        message: String,
    },
}

#[derive(Debug)]
pub struct ReleaseStatusJob {
    pub rx: Receiver<ReleaseStatusMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub branch: String,
}

/// Conventions derived from a checkout's history, used to seed settings the
/// user has not chosen yet.
#[derive(Debug)]
pub enum SettingsSuggestMsg {
    Done {
        language: Option<String>,
        /// Candidate message shapes, most representative first, so the settings
        /// row can be stepped through them instead of offering a single guess.
        shapes: Vec<String>,
    },
    Error(String),
}

#[derive(Debug)]
pub struct SettingsSuggestJob {
    pub rx: Receiver<SettingsSuggestMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
}

#[derive(Debug)]
pub enum CommitLogMsg {
    Done {
        branch: String,
        commits: Vec<Commit>,
    },
    Error {
        branch: String,
        message: String,
    },
}

#[derive(Debug)]
pub struct CommitLogJob {
    pub rx: Receiver<CommitLogMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub branch: String,
}

#[derive(Debug)]
pub struct RefreshJob {
    pub rx: Receiver<RefreshMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub refresh_diff: bool,
}

#[derive(Debug)]
pub enum DiffMsg {
    Done { source: DiffSource, text: String },
}

#[derive(Debug)]
pub struct DiffJob {
    pub rx: Receiver<DiffMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub source: DiffSource,
}

#[derive(Debug)]
pub enum ReviewMsg {
    Done(Box<AssistedReview>),
    Error(String),
}

#[derive(Debug)]
pub struct ReviewJob {
    pub rx: Receiver<ReviewMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
}

#[derive(Debug)]
pub struct ReviewAssistJob {
    pub rx: Receiver<GenMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub node_id: String,
    pub output: String,
    pub spinner: usize,
}

#[derive(Debug)]
pub enum ReviewFlagMsg {
    Started {
        path: String,
        index: usize,
        total: usize,
    },
    Done {
        path: String,
        finding: ReviewStyleFinding,
    },
    Error {
        path: String,
        message: String,
    },
    Finished,
}

#[derive(Debug)]
pub struct ReviewFlagJob {
    pub rx: Receiver<ReviewFlagMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub active_path: Option<String>,
    pub completed: usize,
    pub total: usize,
    pub spinner: usize,
}

#[derive(Debug)]
pub struct ReviewChatJob {
    pub rx: Receiver<GenMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub output: String,
    pub spinner: usize,
}

#[derive(Debug)]
pub enum WorkflowMsg {
    Progress(usize),
    Done(String),
    Error(String),
}

#[derive(Debug)]
pub struct WorkflowJob {
    pub rx: Receiver<WorkflowMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub label: String,
    pub steps: Vec<String>,
    pub current_step: Option<usize>,
}
