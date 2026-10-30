use std::collections::HashSet;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;

use crate::git::{
    Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo, ReleaseBranches, RemoteBranch,
    Worktree,
};

use super::{DiffSource, FlowRun};

mod generation;

pub use generation::{Arrival, CommitDraft, GenMsg, Generation};

/// What a worker's channel held when it was looked at once.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll<M> {
    /// Nothing yet, and the worker is still at it.
    Pending,
    Message(M),
    /// The worker is gone and nothing more will come. After the message that
    /// ends a job this is how a worker says goodbye; before it, the worker
    /// died without finishing — one that panicked, typically.
    Disconnected,
}

thread_local! {
    /// How many times [`poll_once`] has found something on this thread.
    static DELIVERIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many messages (and goodbyes) the workers have delivered to this thread
/// so far. Every job is drained through [`poll_once`], so the loop compares
/// this before and after a pass to tell whether any of them changed something
/// worth redrawing — without every drain having to say so itself.
pub fn deliveries() -> u64 {
    DELIVERIES.with(std::cell::Cell::get)
}

/// Look at `rx` once, without waiting.
pub fn poll_once<M>(rx: &Receiver<M>) -> Poll<M> {
    let poll = match rx.try_recv() {
        Ok(msg) => Poll::Message(msg),
        Err(TryRecvError::Empty) => return Poll::Pending,
        Err(TryRecvError::Disconnected) => Poll::Disconnected,
    };
    DELIVERIES.with(|count| count.set(count.get().wrapping_add(1)));
    poll
}

/// Everything a channel held when it was looked at.
pub struct Drained<M> {
    pub messages: Vec<M>,
    /// Whether the worker is gone. It drops its end of the channel when it
    /// returns, after the message that ends the job — or, when it panicked,
    /// without ever sending one.
    pub disconnected: bool,
}

/// Take every message `rx` holds, without waiting for more.
pub fn drain_receiver<M>(rx: &Receiver<M>) -> Drained<M> {
    let mut messages = Vec::new();
    loop {
        match poll_once(rx) {
            Poll::Message(msg) => messages.push(msg),
            Poll::Pending => {
                return Drained {
                    messages,
                    disconnected: false,
                };
            }
            Poll::Disconnected => {
                return Drained {
                    messages,
                    disconnected: true,
                };
            }
        }
    }
}

/// What to say about work whose worker ended before it answered.
pub fn stopped_unexpectedly(name: &str) -> String {
    format!("{name} stopped unexpectedly")
}

/// A background job: the channel its worker reports on, the worker's handle, and
/// the spinner frame it is drawn at. Lets the drain loop treat every job the
/// same way, whatever else the job carries.
pub trait BackgroundJob {
    type Msg;
    /// What the job is called when it has to be reported on, as in "push
    /// stopped unexpectedly".
    const NAME: &'static str;

    fn rx(&self) -> &Receiver<Self::Msg>;
    fn handle_mut(&mut self) -> &mut Option<JoinHandle<()>>;
    fn spinner_mut(&mut self) -> &mut usize;
}

macro_rules! impl_background_job {
    ($($job:ident => $msg:ty, $name:literal),+ $(,)?) => { $(
        impl BackgroundJob for $job {
            type Msg = $msg;
            const NAME: &'static str = $name;

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
    Generation => GenMsg, "message generation",
    PushJob => PushMsg, "push",
    CheckoutJob => CheckoutMsg, "checkout",
    OperationJob => OperationMsg, "git operation",
    FetchJob => FetchMsg, "fetch",
    RefreshJob => RefreshMsg, "refresh",
    ReleaseStatusJob => ReleaseStatusMsg, "deployment status",
    NestedDetailJob => NestedDetailMsg, "nested branches",
    SettingsSuggestJob => SettingsSuggestMsg, "convention scan",
    CommitLogJob => CommitLogMsg, "commit log",
    DiffJob => DiffMsg, "diff",
    ConflictResolveJob => ConflictResolveMsg, "conflict resolution",
    WorkflowJob => WorkflowMsg, "branch action",
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
    /// Changes something on GitHub and nothing here: a review, a merge, a new
    /// pull request. What the GitHub modal lists is read again afterwards.
    GitHub,
    /// Pushes the current branch and opens a pull request for it. The push is
    /// what keeps it behind a fetch, which [`Self::GitHub`] is not.
    OpenPullRequest,
    /// Clones a repository beside this one, which lg then moves to.
    Clone,
    /// Submits a guided review's notes as a pull request review; the notes
    /// are cleared once GitHub has them, so they are never sent twice.
    SubmitReview,
    /// Adds to, applies or drops from the stash; an open stash list is read
    /// again afterwards.
    Stash,
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
    /// Nothing to fetch from: not worth a status line.
    NoRemotes,
    Error(String),
}

/// What waits for a running fetch to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedAfterFetch {
    Push,
    Pull,
}

impl QueuedAfterFetch {
    pub fn label(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }

    pub fn action(self) -> crate::state::PendingAction {
        match self {
            Self::Push => crate::state::PendingAction::Push,
            Self::Pull => crate::state::PendingAction::Pull,
        }
    }
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

/// How much a refresh reads again. Each scope takes in the ones before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefreshScope {
    /// The current checkout's files, and its diff: what editing a file can
    /// change, and nothing else.
    Files,
    /// Everything git knows about the repository as well: branches, commits,
    /// worktrees, remotes, who commits here.
    Repository,
    /// The repository plus the scan of the workspace for the repositories in
    /// it, which walks the directory tree.
    Workspace,
}

#[derive(Debug)]
pub struct RefreshSnapshot {
    /// What was read. Below [`RefreshScope::Repository`] only `files` (and
    /// `errors`) say anything; the rest is left as it was.
    pub scope: RefreshScope,
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
    /// Who commits here, as `name <email>`. Read with the rest because it is
    /// four git calls, and a snapshot is applied on the thread that draws.
    pub commit_author: Option<String>,
    /// Whether the decorative animations run, which depends on that author.
    pub decorative_animations: bool,
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

/// The branches of the nested repository the tree has expanded, read off the
/// main thread: a repository with a slow remote or thousands of branches used
/// to hold the whole interface still while they were listed.
#[derive(Debug)]
pub struct NestedDetailJob {
    pub rx: Receiver<NestedDetailMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    /// The repository read, relative to the workspace.
    pub path: String,
    /// The repository is being expanded, rather than read again after a push.
    pub opening: bool,
}

#[derive(Debug)]
pub enum NestedDetailMsg {
    Done {
        branches: Vec<Branch>,
        remote_branches: Vec<RemoteBranch>,
    },
    Error(String),
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
    /// Asked for by the user, so its answer replaces saved values in the
    /// modal; an automatic scan only fills a checkout that has none.
    pub requested: bool,
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
    pub scope: RefreshScope,
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

/// How one pass of the local model over the conflicted files is going.
#[derive(Debug)]
pub enum ConflictResolveMsg {
    Started {
        path: String,
        index: usize,
        total: usize,
    },
    /// The file was written back with every conflict in it settled;
    /// `verdicts` says how, conflict by conflict.
    Resolved { path: String, verdicts: Vec<String> },
    /// The file was left exactly as git wrote it, because the local model was
    /// the wrong tool for it. `reason` says which way it fell short.
    Declined { path: String, reason: String },
    /// The pass is over. `declined` carries each path with the reason it was
    /// left alone, so the panel can say why and not only that.
    Finished {
        resolved: Vec<String>,
        declined: Vec<(String, String)>,
    },
}

#[derive(Debug)]
pub struct ConflictResolveJob {
    pub rx: Receiver<ConflictResolveMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub spinner: usize,
    pub active_path: Option<String>,
    pub completed: usize,
    pub total: usize,
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
    /// The branch action being run, kept so the modal can go on drawing the
    /// graph the menu drew and move its marker along with the steps. `None` for
    /// the jobs that are not a branch action, which have no graph to draw.
    pub flow: Option<FlowRun>,
}

#[cfg(test)]
mod tests;
