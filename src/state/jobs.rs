use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;

use crate::git::{
    AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo, ReleaseBranches,
    RemoteBranch, Worktree,
};

use super::{DiffSource, FlowRun};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReviewStyleFinding {
    pub severity: ReviewStyleSeverity,
    pub line: Option<usize>,
    pub reason: String,
}

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
    ReviewJob => ReviewMsg, "review",
    ReviewAssistJob => GenMsg, "review assist",
    ReviewFlagJob => ReviewFlagMsg, "style pass",
    ReviewChatJob => GenMsg, "review chat",
    ConflictResolveJob => ConflictResolveMsg, "conflict resolution",
    WorkflowJob => WorkflowMsg, "branch action",
}

#[derive(Debug)]
pub enum GenMsg {
    Thinking(String),
    Output(String),
    /// What has been sent as output turned out to be reasoning after all — a
    /// closing think tag arrived with no opening one. Consumers drop whatever
    /// they have accumulated and start over.
    Reset,
    /// The finished answer, with what the server said it cost.
    ///
    /// The stats travel with the answer rather than ahead of it because a
    /// consumer polls this channel in batches: sent separately, the stats and
    /// the answer they describe can land on different passes, and the pass that
    /// takes delivery of the answer is the one that has to know whether it was
    /// cut off.
    Done {
        text: String,
        stats: crate::llm::GenStats,
    },
    Error(String),
}

/// A commit message being written, or written, for one checkout. There is a
/// draft per checkout rather than one for the whole app: a message can be
/// left generating in the background and another started in the next
/// checkout, and the two must not be mistaken for each other. While the
/// modal is closed the draft is the sub-line under its checkout in the
/// workspace tree that leads back to it.
#[derive(Debug)]
pub struct CommitDraft {
    /// The checkout the message describes.
    pub dir: String,
    /// The model writing it, gone once it has finished or been cancelled.
    pub generation: Option<Generation>,
    /// The finished message, held until the checkout's modal is opened on
    /// it. Empty while the model is still writing: what has arrived so far
    /// lives in the generation, where the scene can see it stream in.
    pub text: String,
    /// The model has finished and nobody has looked at the result yet.
    pub ready: bool,
}

impl CommitDraft {
    pub fn generating(&self) -> bool {
        self.generation.is_some()
    }
}

#[derive(Debug)]
pub struct Generation {
    pub rx: Receiver<GenMsg>,
    pub handle: Option<JoinHandle<()>>,
    pub output: String,
    pub spinner: usize,
    /// Which of the waiting scenes this generation shows, chosen when it
    /// started so the picture does not change under the reader.
    pub scene: usize,
    /// The words that have lately left the network, still flying in the
    /// scene to their place in the text.
    pub arrivals: Vec<Arrival>,
    /// When the first word left the network on the animation clock; the
    /// scene makes an event of the wait paying off.
    pub first_output_ms: Option<u64>,
    /// The diff that went to the model, prepared for the scene: it is what
    /// flows down the stream into the network.
    pub feed: crate::panel::commit_art::Feed,
    /// What the model has sent that has not left the network yet, a word to
    /// an entry. A model may send a word at a time or a line at a time;
    /// either way the words fly out one after another.
    pub queued: VecDeque<String>,
    /// When the next queued word may leave, on the animation clock.
    pub next_word_ms: u64,
    /// The gap between words once the model has finished, fixed then so
    /// what is left goes out at one steady pace.
    pub finish_gap_ms: Option<u64>,
    /// When the first of the message came from the model, on the animation
    /// clock: the words are held back for a while from then.
    pub first_chunk_ms: Option<u64>,
    /// The finished message, held back until the last word has landed so
    /// the editor does not take the text out from under the words in flight.
    pub finished: Option<(String, crate::llm::GenStats)>,
}

/// How long the words waiting are spread over while the model is still
/// writing. A model can go quiet for a second or more mid-message and then
/// send the rest at once; spreading what is queued slows the words down as
/// the queue runs low, so the text keeps moving through the quiet rather
/// than stopping dead, and a burst is caught up quickly.
const WORD_SPREAD_MS: u64 = 1_200;
/// The longest gap between words, with one word left waiting.
const MAX_WORD_GAP_MS: u64 = 220;
/// How long what is left is given to go out once the model has finished.
const FINISH_MS: u64 = 1_200;
/// The longest gap between words once the model has finished.
const FINISH_WORD_GAP_MS: u64 = 80;
/// The shortest gap between words, however far behind they are.
const MIN_WORD_GAP_MS: u64 = 10;
/// Words queued before the first of them goes out. Claude sends the opening
/// word or two, goes quiet for up to a second, then sends the rest; starting
/// on the opening alone would show it and then stand still. Held back, the
/// quiet is spent in the scene, which looks like the network still working.
const START_WORDS: usize = 8;
/// How long the opening is held back at most, for a message that is short or
/// slow to come.
const START_WAIT_MS: u64 = 1_500;

impl Generation {
    pub fn new(
        rx: Receiver<GenMsg>,
        handle: JoinHandle<()>,
        feed: crate::panel::commit_art::Feed,
    ) -> Self {
        Self {
            rx,
            handle: Some(handle),
            output: String::new(),
            spinner: 0,
            scene: crate::panel::commit_art::fresh_seed(),
            arrivals: Vec::new(),
            first_output_ms: None,
            feed,
            queued: VecDeque::new(),
            next_word_ms: 0,
            finish_gap_ms: None,
            first_chunk_ms: None,
            finished: None,
        }
    }

    /// Take in a chunk of the message. The last queued word may be the
    /// first half of one this chunk finishes, so it is cut up again along
    /// with it.
    pub fn receive(&mut self, chunk: &str) {
        let mut text = self.queued.pop_back().unwrap_or_default();
        text.push_str(chunk);
        self.queued.extend(words(&text));
    }

    /// Forget everything received: what came was reasoning, not the message.
    pub fn restart(&mut self) {
        self.output.clear();
        self.arrivals.clear();
        self.queued.clear();
        self.first_output_ms = None;
        self.finish_gap_ms = None;
        self.first_chunk_ms = None;
    }

    /// Let out the words that are due by `now`. A last queued word with no
    /// space after it waits for more, or for the model to finish, since the
    /// rest of it may be on the way. With `animate` off, everything ready
    /// goes at once.
    pub fn release(&mut self, now: u64, animate: bool) {
        if !self.queued.is_empty() {
            self.first_chunk_ms.get_or_insert(now);
        }
        let starting = self.first_output_ms.is_none()
            && self.finished.is_none()
            && self.queued.len() < START_WORDS
            && self
                .first_chunk_ms
                .is_some_and(|first| now.saturating_sub(first) < START_WAIT_MS);
        if animate && starting {
            return;
        }
        let whole = self.finished.is_some()
            || self
                .queued
                .back()
                .is_some_and(|word| word.ends_with(char::is_whitespace));
        let mut ready = if whole {
            self.queued.len()
        } else {
            self.queued.len().saturating_sub(1)
        };
        if self.finished.is_some() && self.finish_gap_ms.is_none() {
            self.finish_gap_ms =
                Some((FINISH_MS / ready.max(1) as u64).clamp(MIN_WORD_GAP_MS, FINISH_WORD_GAP_MS));
        }
        // After a pause the clock has run on; the words that come next start
        // from now rather than all leaving at once to make up the time.
        let mut at = if now.saturating_sub(self.next_word_ms) > FINISH_WORD_GAP_MS {
            now
        } else {
            self.next_word_ms
        };
        self.arrivals
            .retain(|a| now.saturating_sub(a.at_ms) < crate::panel::commit_art::FLIGHT_TOTAL_MS);
        while ready > 0 && (!animate || at <= now) {
            let Some(word) = self.queued.pop_front() else {
                break;
            };
            let at_ms = if animate { at } else { now };
            self.arrivals.push(Arrival {
                start: self.output.chars().count(),
                len: word.chars().count(),
                at_ms,
            });
            self.output.push_str(&word);
            self.first_output_ms.get_or_insert(at_ms);
            at += self.finish_gap_ms.unwrap_or_else(|| {
                (WORD_SPREAD_MS / ready as u64).clamp(MIN_WORD_GAP_MS, MAX_WORD_GAP_MS)
            });
            ready -= 1;
        }
        self.next_word_ms = at;
    }

    /// Whether every word has left the network and landed in the text.
    pub fn settled(&self, now: u64) -> bool {
        self.queued.is_empty()
            && self
                .arrivals
                .iter()
                .all(|a| now.saturating_sub(a.at_ms) >= crate::panel::commit_art::FLIGHT_MS)
    }
}

/// `text` cut after each word, so every piece is one word with the space
/// that follows it.
fn words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut after_space = false;
    for c in text.chars() {
        let starts_word = !c.is_whitespace()
            && after_space
            && words
                .last()
                .is_some_and(|word| word.chars().any(|c| !c.is_whitespace()));
        match words.last_mut() {
            Some(word) if !starts_word => word.push(c),
            _ => words.push(c.to_string()),
        }
        after_space = c.is_whitespace();
    }
    words
}

/// One word of the message as it left the model: where it sits in the
/// output, and when it left on the animation clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrival {
    /// First character of the word in the output.
    pub start: usize,
    pub len: usize,
    pub at_ms: u64,
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

/// A Claude Code session reviewing the branch. It reports by writing a file,
/// which is read back each frame until the session goes away.
#[derive(Debug)]
pub struct ReviewAgentJob {
    pub session: crate::session::SessionId,
    pub findings: std::path::PathBuf,
    /// What the file said when last read, so the tree is only touched when it
    /// has changed.
    pub seen: String,
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
mod tests {
    use super::*;

    const FRAME_MS: u64 = crate::config::ANIMATION_FRAME_MS;

    fn generation() -> Generation {
        let (_tx, rx) = std::sync::mpsc::channel();
        Generation::new(rx, std::thread::spawn(|| {}), Default::default())
    }

    /// Run the clock from `from` to `to` a frame at a time, as the loop does.
    fn run(g: &mut Generation, from: u64, to: u64) {
        let mut now = from;
        while now <= to {
            g.release(now, true);
            now += FRAME_MS;
        }
    }

    fn finish(g: &mut Generation) {
        g.finished = Some((String::new(), Default::default()));
    }

    /// The words in flight at `now`, as they read in the output.
    fn in_flight(g: &Generation, now: u64) -> Vec<String> {
        g.arrivals
            .iter()
            .filter(|a| now.saturating_sub(a.at_ms) < crate::panel::commit_art::FLIGHT_MS)
            .map(|a| g.output.chars().skip(a.start).take(a.len).collect())
            .collect()
    }

    /// A model that sends a line at once has it written out a word at a
    /// time, the way one sending a word at a time would.
    #[test]
    fn a_line_from_the_model_comes_out_a_word_at_a_time() {
        let mut g = generation();
        g.receive("feat: accept a name argument");
        finish(&mut g);

        g.release(1_000, true);
        assert_eq!(g.output, "feat: ");

        run(&mut g, 1_000, 3_000);
        assert_eq!(g.output, "feat: accept a name argument");
    }

    /// Half a word in one chunk and the rest in the next fly as one word.
    #[test]
    fn a_word_split_across_chunks_flies_whole() {
        let mut g = generation();
        g.receive("word-fre");
        g.receive("quency tool");
        finish(&mut g);
        run(&mut g, 1_000, 1_000 + 2 * MAX_WORD_GAP_MS);

        let flown: Vec<String> = g
            .arrivals
            .iter()
            .map(|a| g.output.chars().skip(a.start).take(a.len).collect())
            .collect();
        assert!(
            flown.iter().any(|w| w.trim() == "word-frequency"),
            "{flown:?}"
        );
    }

    /// Until the model says more, the word it stopped on may not be whole.
    #[test]
    fn the_last_word_waits_for_more_until_the_model_finishes() {
        let mut g = generation();
        g.receive("add a READ");
        run(&mut g, 1_000, 3_000);
        assert_eq!(g.output, "add a ");

        g.receive("ME file");
        finish(&mut g);
        run(&mut g, 3_000, 5_000);
        assert_eq!(g.output, "add a README file");
    }

    /// The editor takes the message over only once nothing is left in the
    /// air, so no word is cut off on its way into the text.
    #[test]
    fn a_finished_message_settles_once_its_last_word_has_landed() {
        let mut g = generation();
        g.receive("fix: handle empty input");
        finish(&mut g);
        assert!(!g.settled(1_000), "nothing has gone out yet");

        let mut now = 1_000;
        while !g.queued.is_empty() {
            g.release(now, true);
            now += FRAME_MS;
        }
        assert!(!in_flight(&g, now).is_empty());
        assert!(!g.settled(now), "the last word is still flying");

        now += crate::panel::commit_art::FLIGHT_MS;
        assert!(in_flight(&g, now).is_empty());
        assert!(g.settled(now));
    }

    /// A long message sent in one go does not keep the reader waiting long
    /// after the model is done.
    #[test]
    fn a_long_backlog_is_written_out_quickly() {
        let mut g = generation();
        g.receive(&"word ".repeat(120));
        finish(&mut g);
        run(&mut g, 1_000, 1_000 + 2 * FINISH_MS);
        assert!(g.queued.is_empty());
        assert_eq!(g.output, "word ".repeat(120));
    }

    /// After a pause the words do not all go at once to make up the time.
    #[test]
    fn words_after_a_pause_still_go_one_at_a_time() {
        let mut g = generation();
        g.receive("one ");
        g.receive("two");
        run(&mut g, 1_000, 1_100);
        g.receive(" three four five six");
        finish(&mut g);
        g.release(10_000, true);
        assert_eq!(in_flight(&g, 10_000).len(), 1);
    }

    /// Claude can go quiet for over a second mid-message and then send the
    /// rest at once. What is already queued is spread over the quiet, so
    /// the text keeps moving instead of stopping dead until the burst.
    #[test]
    fn the_text_keeps_moving_through_a_pause_in_the_stream() {
        let mut g = generation();
        g.receive(&"word ".repeat(20));
        let quiet = 1_400;
        let mut last_word_at = 0;
        let mut longest_still = 0;
        let mut now = 1_000;
        while now <= 1_000 + quiet {
            let before = g.output.len();
            g.release(now, true);
            if g.output.len() > before {
                longest_still = longest_still.max(now - last_word_at.max(1_000));
                last_word_at = now;
            }
            now += FRAME_MS;
        }
        longest_still = longest_still.max(1_000 + quiet - last_word_at);
        assert!(
            longest_still <= 300,
            "the text stood still for {longest_still} ms"
        );

        g.receive(&"more ".repeat(60));
        finish(&mut g);
        run(&mut g, now, now + FINISH_MS + MAX_WORD_GAP_MS + FRAME_MS);
        assert!(g.queued.is_empty(), "the burst is caught up quickly");
    }

    /// Claude sends the opening word or two, goes quiet for about a second,
    /// then sends the rest. The text does not start on the opening alone and
    /// then stand still: once it starts it keeps moving to the end.
    #[test]
    fn an_opening_followed_by_a_pause_does_not_stall_the_text() {
        let mut g = generation();
        g.receive("feat(commit): fly");
        let mut now = 1_000;
        while now < 2_200 {
            g.release(now, true);
            now += FRAME_MS;
        }
        g.receive(&" streamed words in one at a time".repeat(4));
        finish(&mut g);

        let mut last_word_at = None;
        let mut longest_still = 0;
        let mut seen = 0;
        while !g.settled(now) {
            let before = g.output.len();
            g.release(now, true);
            if g.output.len() > before {
                if seen > 0
                    && let Some(at) = last_word_at
                {
                    longest_still = std::cmp::max(longest_still, now - at);
                }
                seen = g.output.len();
                last_word_at = Some(now);
            }
            now += FRAME_MS;
        }
        assert!(
            longest_still <= 300,
            "the text stood still for {longest_still} ms"
        );
    }

    /// A short message that never gets going still shows up.
    #[test]
    fn a_lone_opening_is_shown_after_a_while() {
        let mut g = generation();
        g.receive("fix: typo ");
        run(
            &mut g,
            1_000,
            1_000 + START_WAIT_MS + MAX_WORD_GAP_MS + FRAME_MS,
        );
        assert_eq!(g.output, "fix: typo ");
    }

    #[test]
    fn with_animations_off_everything_shows_at_once() {
        let mut g = generation();
        g.receive("docs: explain the flag");
        finish(&mut g);
        g.release(1_000, false);
        assert_eq!(g.output, "docs: explain the flag");
        assert!(g.queued.is_empty());
    }

    #[test]
    fn reasoning_taken_back_leaves_nothing_queued() {
        let mut g = generation();
        g.receive("thinking about it ");
        run(&mut g, 1_000, 1_100);
        g.restart();
        finish(&mut g);
        run(&mut g, 1_100, 2_000);
        assert!(g.output.is_empty());
    }
}
