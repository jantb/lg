//! A guided review: the change walked one file at a time, with the model's
//! read of each file beside it, notes pinned to lines, and the code editable
//! on the way through; then a final read of the whole change across files.
//!
//! The walk runs over the branch on screen — against main, or against where it
//! was last pushed — in the working tree, so an edit made during the review is
//! part of it the moment it is saved; or over a pull request as GitHub shows
//! it, whose notes are submitted as one review with inline comments. Which steps were looked at and every note
//! written are kept in the git directory, so leaving and coming back resumes
//! where the review stopped.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;

use crate::git::guided::{GuidedFile, GuidedNote, GuidedProgress, WalkMode};
use crate::panel::text_input::TextInput;
use crate::state::{DiffViewMode, GenMsg};

mod assist;
mod chat;
mod context;
mod draw;
mod findings;
mod found;
mod keys;
mod layout;
mod load;
mod mouse;
mod navigate;
mod notes;
mod plan;
mod poll;
mod progress;
mod tree;
mod work;

pub use chat::{chat_has_keyboard, chat_on_screen, fit_chat};
pub use draw::render;
pub use keys::{handle_key, handle_paste};
pub use load::{open_branch, open_pull_request, reload};
pub use mouse::handle_mouse;
pub use notes::after_submit;
pub use poll::poll;

/// The step before the first file: what the change is for.
pub const OVERVIEW: usize = 0;
const OVERVIEW_KEY: &str = "overview";
/// What the chat is about on the final step, where it is no one file.
pub const WHOLE_CHANGE: &str = "the whole change";
/// Source lines either side of a hunk that travel with it to the model.
const SOURCE_CONTEXT_LINES: usize = 25;
/// How many requests for commentary may run at once.
const CONCURRENT_LOCAL: usize = 1;
const CONCURRENT_CLAUDE: usize = 3;
/// How much of the diff the plan and the final review are shown; a file past
/// it is named but left out, so one huge file cannot crowd out the rest.
const WHOLE_DIFF_BYTES: usize = 120_000;
/// How much of the rest of a file's concern travels with it to the model.
const CONCERN_CONTEXT_BYTES: usize = 40_000;
/// How many reads may have their questions out with Claude at once, so
/// reading a whole concern does not start a run per file.
const CONCURRENT_CHECKS: usize = 2;

/// What is being reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Branch {
        branch: String,
        base_ref: String,
    },
    /// What is not pushed yet: unpushed commits and uncommitted edits, against
    /// where the branch was last pushed. The walk offered on main, where the
    /// branch has nothing against itself.
    Unpushed {
        branch: String,
        /// Where the branch was last pushed, once the walk has read it.
        base_ref: String,
    },
    /// The whole codebase as it stands, every file read as if it were new:
    /// what there is to review when nothing is set against it.
    Codebase {
        branch: String,
    },
    PullRequest {
        number: u64,
        title: String,
        head: String,
        base: String,
        /// The commit inline comments are pinned to.
        commit: String,
        /// The pull request's branch is the one checked out here, so its files
        /// can be opened, edited and fixed in place.
        local: bool,
    },
}

impl Source {
    pub fn is_pull_request(&self) -> bool {
        matches!(self, Self::PullRequest { .. })
    }

    /// Whether the files on disk are the ones being reviewed.
    pub fn editable(&self) -> bool {
        match self {
            Self::Branch { .. } | Self::Unpushed { .. } | Self::Codebase { .. } => true,
            Self::PullRequest { local, .. } => *local,
        }
    }

    fn progress_key(&self) -> String {
        match self {
            Self::Branch { branch, .. } => format!("branch-{branch}"),
            Self::Unpushed { branch, .. } => format!("unpushed-{branch}"),
            Self::Codebase { branch } => format!("codebase-{branch}"),
            Self::PullRequest { number, .. } => format!("pr-{number}"),
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Branch { branch, base_ref } => format!("{branch} vs {base_ref}"),
            Self::Unpushed { branch, base_ref } if base_ref.is_empty() => {
                format!("what is not pushed on {branch}")
            }
            Self::Unpushed { branch, base_ref } => {
                format!("what is not pushed on {branch} (vs {base_ref})")
            }
            Self::Codebase { branch } => format!("the whole codebase on {branch}"),
            Self::PullRequest {
                number,
                title,
                head,
                base,
                ..
            } => format!("#{number} {title} ({head} \u{2192} {base})"),
        }
    }
}

/// What the model said, or is saying, about one step.
#[derive(Debug, Clone, Default)]
pub struct Commentary {
    pub text: String,
    pub thinking: bool,
    pub done: bool,
    /// Claude is reading the checkout to settle the questions the answer
    /// raised, which are held back until it has.
    pub checking: bool,
    pub error: Option<String>,
    /// When it was asked for, and how long the prompt was, to tell how far
    /// the model has got reading it.
    pub started: Option<std::time::Instant>,
    pub prompt_bytes: usize,
}

impl Commentary {
    /// The verdict line the step prompt ends on, once the answer is in.
    pub fn verdict(&self) -> Option<Verdict> {
        if !self.done {
            return None;
        }
        let last = self
            .text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())?;
        let word = last.trim().strip_prefix("Verdict:")?.trim().to_lowercase();
        Some(match word.trim_matches(|c: char| !c.is_alphabetic()) {
            "ok" => Verdict::Ok,
            "nit" => Verdict::Nit,
            "issue" => Verdict::Issue,
            _ => return None,
        })
    }
}

/// Ordered so the worst of several is their `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    Nit,
    Issue,
}

/// A question asked about a step, and the answer so far.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub question: String,
    pub answer: Commentary,
}

/// One request to the model in flight.
struct Ask {
    /// The commentary it fills in: a step key, or a step key plus the index of
    /// the question being answered.
    target: Target,
    rx: Receiver<GenMsg>,
}

/// A step's questions out with Claude to be settled in the code.
struct Check {
    key: String,
    /// Put back as they were if the check fails.
    questions: Vec<String>,
    answer: CheckAnswer,
}

/// Who settles a step's questions: the review chat when one is open, since it
/// is already reading the code with the reviewer, else a headless run.
enum CheckAnswer {
    Headless(Receiver<Result<String, String>>),
    /// The file the chat was told to write its answer to.
    Chat(std::path::PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Commentary(String),
    Answer(String, usize),
}

/// What the keyboard is doing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Browse,
    /// Writing a note on the cursor line; `editing` is the note being changed.
    Note {
        text: TextInput,
        editing: Option<usize>,
    },
    /// Asking the model about the step on screen.
    Question {
        text: TextInput,
    },
    /// Telling Claude what to change.
    Fix {
        text: TextInput,
        all_notes: bool,
    },
    /// Every note, and for a pull request the review they are submitted as.
    Summary {
        event: usize,
        body: TextInput,
    },
    Chat,
}

/// The loaded walk: everything but the receivers that feed it.
pub struct Loaded {
    pub source: Source,
    pub steps: Vec<GuidedFile>,
    pub overview: Overview,
}

/// `brief` leaves out the line counts, so an edit does not change the cached step prompt.
pub struct Overview {
    pub text: String,
    pub brief: String,
}

/// A guided review in progress.
pub struct Guided {
    pub source: Source,
    pub steps: Vec<GuidedFile>,
    pub overview: String,
    brief: String,
    /// Pinned to the walk's first brief, so the cached step prompt never changes.
    shared_brief: Option<String>,
    /// Which step is on screen: [`OVERVIEW`], a file counted from one, or
    /// [`Guided::final_step`].
    pub current: usize,
    /// The line of the file's diff the cursor is on.
    pub cursor: usize,
    pub side_scroll: u16,
    /// How far the side pane could scroll when it was last drawn, so a scroll
    /// key moves from where the pane really is rather than from a "follow the
    /// bottom" request past the end.
    pub side_max: std::cell::Cell<u16>,
    pub progress: GuidedProgress,
    pub commentary: HashMap<String, Commentary>,
    pub exchanges: HashMap<String, Vec<Exchange>>,
    pub mode: Mode,
    pub loading: Option<Receiver<Result<Loaded, String>>>,
    /// A Claude Code run applying a fix, and what it was asked to change.
    pub fixing: Option<(Receiver<Result<String, String>>, String)>,
    /// The pull request being walked, as the list had it, to read it again.
    pull_request: Option<crate::github::PullRequest>,
    asks: Vec<Ask>,
    checks: Vec<Check>,
    /// Steps whose questions wait for a check to free up, by key.
    check_queue: Vec<String>,
    /// Files the reviewer asked to have read ahead of the walk, by path.
    read_wanted: Vec<String>,
    /// Steps whose commentary was asked for and failed, so they are not asked
    /// again every frame.
    failed: HashSet<String>,
    /// The model's last read of each file `r` sent back, for the next read to say what is fixed.
    earlier: HashMap<String, String>,
    /// The note `x` was pressed on once; pressing it again deletes it.
    pub(super) delete_armed: Option<usize>,
    /// Reset was asked for once; asking again resets the review.
    pub reset_armed: bool,
    /// The step list's first row shown, once the wheel has scrolled it; until
    /// then, and after any move by key, the list follows the step on screen.
    pub list_scroll: Option<usize>,
    /// How far the whole changeset is scrolled on the final step; the draw
    /// moves it to bring a newly picked finding into view.
    pub final_scroll: std::cell::Cell<u16>,
    /// How far it could scroll when it was last drawn, as [`Guided::side_max`].
    pub final_max: std::cell::Cell<u16>,
    /// The final review's finding picked on the final step, by its place
    /// among those the review cites.
    pub final_pick: usize,
    /// The pick moved since the last draw, which scrolls the whole change to it.
    pub final_reveal: std::cell::Cell<bool>,
    pub chat: Option<crate::session::SessionId>,
    pub chat_file: Option<String>,
    chat_owed: Vec<(String, ChatNews)>,
}

/// What the chat is told about a file once lg has read all of it.
/// Ordered so a redo's news replaces a plain read's still waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChatNews {
    FileRead,
    Redone,
}

impl Guided {
    pub(crate) fn new(source: Source) -> Self {
        let progress = crate::git::guided::load_progress(&source.progress_key());
        Self {
            source,
            steps: Vec::new(),
            overview: String::new(),
            brief: String::new(),
            shared_brief: None,
            current: OVERVIEW,
            cursor: 0,
            side_scroll: 0,
            side_max: std::cell::Cell::new(0),
            progress,
            commentary: HashMap::new(),
            exchanges: HashMap::new(),
            mode: Mode::Browse,
            loading: None,
            fixing: None,
            pull_request: None,
            asks: Vec::new(),
            checks: Vec::new(),
            check_queue: Vec::new(),
            read_wanted: Vec::new(),
            failed: HashSet::new(),
            earlier: HashMap::new(),
            delete_armed: None,
            reset_armed: false,
            list_scroll: None,
            final_scroll: std::cell::Cell::new(0),
            final_max: std::cell::Cell::new(0),
            final_pick: 0,
            final_reveal: std::cell::Cell::new(false),
            chat: None,
            chat_file: None,
            chat_owed: Vec::new(),
        }
    }

    pub fn step(&self) -> Option<&GuidedFile> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.steps.get(index))
    }

    /// The key the step on screen is known by in the commentary and progress.
    pub fn step_key(&self) -> String {
        match self.step() {
            Some(step) => step.key(),
            None if self.on_final() => self.final_key(),
            None => OVERVIEW_KEY.to_string(),
        }
    }

    /// How many files the walk has.
    pub fn total(&self) -> usize {
        self.steps.len()
    }

    /// The step after the last file: the whole change, read across files.
    pub fn final_step(&self) -> usize {
        self.total() + 1
    }

    pub fn on_final(&self) -> bool {
        self.current == self.final_step()
    }

    /// The model's plan for the walk, once its opening read is in.
    pub fn plan(&self) -> Option<plan::Plan> {
        let commentary = self
            .commentary
            .get(OVERVIEW_KEY)
            .filter(|c| c.done && c.error.is_none())?;
        let paths: Vec<&str> = self.steps.iter().map(|step| step.path.as_str()).collect();
        Some(plan::read(&commentary.text, &paths))
    }

    pub fn walk(&self) -> WalkMode {
        self.progress.walk
    }

    /// The files, counted from one, in the order the walk visits them: path
    /// order until the plan is in.
    pub fn walk_order(&self) -> Vec<usize> {
        let Some(plan) = self.plan() else {
            return (1..=self.total()).collect();
        };
        let paths: Vec<String> = match self.walk() {
            WalkMode::Concerns | WalkMode::Issues => plan
                .concerns
                .iter()
                .flat_map(|concern| {
                    let mut paths = concern.paths.clone();
                    paths.sort();
                    paths
                })
                .collect(),
            WalkMode::Map => plan.order.clone(),
            // Band by band, each band's files as its tree lists them.
            WalkMode::Triage => {
                let mut paths = plan.order.clone();
                paths.sort_by(|a, b| {
                    triage_rank(&plan, a)
                        .cmp(&triage_rank(&plan, b))
                        .then(a.cmp(b))
                });
                paths
            }
        };
        paths
            .iter()
            .filter_map(|path| self.steps.iter().position(|step| step.path == *path))
            .map(|index| index + 1)
            .collect()
    }

    /// Whether the model has answered for `step`, its questions settled or not.
    pub fn is_read(&self, step: &GuidedFile) -> bool {
        self.commentary
            .get(&step.key())
            .is_some_and(|c| c.done || c.checking)
    }

    /// How many of `paths` the model has read, and how many there are.
    pub fn read_of(&self, paths: &[String]) -> (usize, usize) {
        let steps: Vec<&GuidedFile> = self
            .steps
            .iter()
            .filter(|step| paths.contains(&step.path))
            .collect();
        let read = steps.iter().filter(|step| self.is_read(step)).count();
        (read, steps.len())
    }

    /// Whether the reviewer asked for `path` to be read ahead of the walk.
    pub fn wants_read(&self, path: &str) -> bool {
        self.read_wanted.iter().any(|wanted| wanted == path)
    }

    /// Whether every file of `paths` was asked for ahead of the walk and not
    /// all of them are in yet.
    pub fn reading_ahead(&self, paths: &[String]) -> bool {
        let (read, total) = self.read_of(paths);
        read < total && paths.iter().all(|path| self.read_wanted.contains(path))
    }

    /// How far the plan has got while it is being read: the diff being read,
    /// then the files it has placed so far.
    pub fn planning(&self) -> Option<Progress> {
        let commentary = self.commentary.get(OVERVIEW_KEY).filter(|c| !c.done)?;
        let named = self
            .steps
            .iter()
            .filter(|step| commentary.text.contains(&step.path))
            .count();
        Some(before_writing(commentary).unwrap_or(Progress::Placing {
            named,
            total: self.total(),
        }))
    }

    /// How far the final review has got: waiting its turn while the walk is
    /// on it, the change being read, then the findings written so far.
    pub fn final_progress(&self) -> Option<Progress> {
        let key = self.final_key();
        match self.commentary.get(&key) {
            None if self.on_final() && !self.failed.contains(&key) => Some(Progress::Queued {
                ahead: self
                    .asks
                    .iter()
                    .filter(|ask| matches!(ask.target, Target::Commentary(_)))
                    .count(),
            }),
            Some(commentary) if !commentary.done => {
                let reading = findings::read(&commentary.text);
                Some(before_writing(commentary).unwrap_or(Progress::Finding {
                    so_far: reading.cited.len() + reading.findings.len(),
                }))
            }
            _ => None,
        }
    }

    /// The step after `current` in the walk: the overview leads to the first
    /// file, the last file to the final review.
    pub fn step_after(&self, current: usize) -> usize {
        let order = self.walk_order();
        let next = match order.iter().position(|&step| step == current) {
            Some(at) => order.get(at + 1),
            None if current == OVERVIEW => order.first(),
            None => None,
        };
        next.copied().unwrap_or(self.final_step())
    }

    pub fn step_before(&self, current: usize) -> usize {
        let order = self.walk_order();
        let previous = match order.iter().position(|&step| step == current) {
            Some(at) => at.checked_sub(1).map(|at| order[at]),
            None if current == self.final_step() => order.last().copied(),
            None => None,
        };
        previous.unwrap_or(OVERVIEW)
    }

    /// Changes with the change, so an edit has the final review read again.
    pub fn final_key(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for step in &self.steps {
            step.key().hash(&mut hasher);
        }
        format!("final#{:016x}", hasher.finish())
    }

    pub fn is_reviewed(&self, step: &GuidedFile) -> bool {
        self.progress.reviewed.contains(&step.key())
    }

    pub fn reviewed_count(&self) -> usize {
        self.steps.iter().filter(|s| self.is_reviewed(s)).count()
    }

    /// Whether a text field is open, which wants pastes delivered whole.
    pub fn typing(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    pub fn working(&self) -> bool {
        self.loading.is_some()
            || self.fixing.is_some()
            || !self.asks.is_empty()
            || !self.checks.is_empty()
            || !self.check_queue.is_empty()
    }

    /// The notes that belong on `step`: those written on it, and those pinned
    /// to a line it still shows.
    pub fn notes_for(&self, step: &GuidedFile) -> Vec<(usize, &GuidedNote)> {
        let key = step.key();
        self.progress
            .notes
            .iter()
            .enumerate()
            .filter(|(_, note)| {
                note.path == step.path
                    && (note.step == key
                        || step
                            .lines
                            .iter()
                            .any(|line| line.anchor() == Some((note.line, note.side))))
            })
            .collect()
    }

    /// The note pinned to the line under the cursor, if there is one.
    pub fn note_at_cursor(&self) -> Option<usize> {
        let step = self.step()?;
        let anchor = step.lines.get(self.cursor)?.anchor()?;
        self.progress
            .notes
            .iter()
            .position(|note| note.path == step.path && (note.line, note.side) == anchor)
    }

    fn visit_order(&self, mode: DiffViewMode) -> Vec<usize> {
        self.step()
            .map(|step| layout::visit_order(step, &layout::rows(step, mode)))
            .unwrap_or_default()
    }

    fn still_reading(&self, ai: bool) -> bool {
        let key = self.step_key();
        match self.commentary.get(&key) {
            Some(commentary) => !commentary.done,
            None => ai && !self.failed.contains(&key),
        }
    }

    fn save(&self) -> Result<(), String> {
        crate::git::guided::save_progress(&self.source.progress_key(), &self.progress)
            .map_err(|e| format!("could not save review progress: {e:#}"))
    }
}

/// A finding not yet ticked off, in the file it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFinding {
    pub step: usize,
    /// Its line's index in the file's diff, when the diff shows that line.
    pub at: Option<usize>,
    pub finding: findings::Finding,
    /// Raised by the final review of the whole change, not the file's own read.
    pub whole: bool,
}

/// A long read on its way: the plan, or the final review of the whole change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Not sent yet: this many reads have the model's turns ahead of it.
    Queued {
        ahead: usize,
    },
    /// Sent, and the model is reading the prompt, which should take
    /// `expected` at the speed it read the last one.
    Reading {
        elapsed: std::time::Duration,
        expected: Option<std::time::Duration>,
    },
    Thinking {
        elapsed: std::time::Duration,
    },
    /// The plan being written: how many of the files it has placed.
    Placing {
        named: usize,
        total: usize,
    },
    /// The final review being written: its findings so far.
    Finding {
        so_far: usize,
    },
}

/// Where a read is before the model writes any of its answer.
fn before_writing(commentary: &Commentary) -> Option<Progress> {
    if !commentary.text.trim().is_empty() {
        return None;
    }
    let elapsed = commentary
        .started
        .map(|started| started.elapsed())
        .unwrap_or_default();
    Some(if commentary.thinking {
        Progress::Thinking { elapsed }
    } else {
        Progress::Reading {
            elapsed,
            expected: crate::llm::expected_prefill_for(commentary.prompt_bytes),
        }
    })
}

/// Riskiest first; files safe to skim after every other.
pub(super) fn triage_rank(plan: &plan::Plan, path: &str) -> usize {
    if plan.skim.contains_key(path) {
        return 4;
    }
    match plan.risk.get(path).map(|(risk, _)| *risk) {
        Some(plan::Risk::High) => 0,
        Some(plan::Risk::Medium) => 1,
        Some(plan::Risk::Low) => 2,
        None => 3,
    }
}

// ─── Opening and loading ─────────────────────────────────────────────────────

// ─── Polling ─────────────────────────────────────────────────────────────────

// ─── Acting ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
