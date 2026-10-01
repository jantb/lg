//! A guided review: the change walked one hunk at a time, with the model's
//! read of each hunk beside it, notes pinned to lines, and the code editable
//! on the way through.
//!
//! The walk runs over either the branch on screen against main — the working
//! tree, so an edit made during the review is part of it the moment it is
//! saved — or a pull request as GitHub shows it, whose notes are submitted as
//! one review with inline comments. Which steps were looked at and every note
//! written are kept in the git directory, so leaving and coming back resumes
//! where the review stopped.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::git::guided::{GuidedHunk, GuidedNote, GuidedProgress, Side};
use crate::state::{AppState, GenMsg, Modal, PendingAction};

mod draw;
mod keys;

pub use draw::render;
pub use keys::{handle_key, handle_paste};

/// The step before the first hunk: what the change is for.
pub const OVERVIEW: usize = 0;
const OVERVIEW_KEY: &str = "overview";
/// Source lines either side of a hunk that travel with it to the model.
const SOURCE_CONTEXT_LINES: usize = 25;
/// How many steps ahead the model reads, so the next step's commentary is
/// usually waiting by the time the reviewer gets there.
const LOOKAHEAD_LOCAL: usize = 1;
const LOOKAHEAD_CLAUDE: usize = 3;

/// What is being reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Branch {
        branch: String,
        base_ref: String,
    },
    /// What is not committed yet on the branch: the walk offered on main,
    /// where the branch has nothing against itself.
    Changes {
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
            Self::Branch { .. } | Self::Changes { .. } => true,
            Self::PullRequest { local, .. } => *local,
        }
    }

    fn progress_key(&self) -> String {
        match self {
            Self::Branch { branch, .. } => format!("branch-{branch}"),
            Self::Changes { branch } => format!("changes-{branch}"),
            Self::PullRequest { number, .. } => format!("pr-{number}"),
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Branch { branch, base_ref } => format!("{branch} vs {base_ref}"),
            Self::Changes { branch } => format!("uncommitted changes on {branch}"),
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
    pub error: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        text: String,
        editing: Option<usize>,
    },
    /// Asking the model about the step on screen.
    Question { text: String },
    /// Telling Claude what to change.
    Fix { text: String, all_notes: bool },
    /// Every note, and for a pull request the review they are submitted as.
    Summary { event: usize, body: String },
}

/// The loaded walk: everything but the receivers that feed it.
pub struct Loaded {
    pub source: Source,
    pub steps: Vec<GuidedHunk>,
    /// Commits and the file list, which every step's prompt opens with.
    pub overview: String,
}

/// A guided review in progress.
pub struct Guided {
    pub source: Source,
    pub steps: Vec<GuidedHunk>,
    pub overview: String,
    /// Which step is on screen: [`OVERVIEW`], or a hunk counted from one.
    pub current: usize,
    /// The line of the hunk the cursor is on.
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
    /// Steps whose commentary was asked for and failed, so they are not asked
    /// again every frame.
    failed: HashSet<String>,
}

impl Guided {
    fn new(source: Source) -> Self {
        let progress = crate::git::guided::load_progress(&source.progress_key());
        Self {
            source,
            steps: Vec::new(),
            overview: String::new(),
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
            failed: HashSet::new(),
        }
    }

    pub fn step(&self) -> Option<&GuidedHunk> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.steps.get(index))
    }

    /// The key the step on screen is known by in the commentary and progress.
    pub fn step_key(&self) -> String {
        self.step()
            .map(GuidedHunk::key)
            .unwrap_or_else(|| OVERVIEW_KEY.to_string())
    }

    pub fn total(&self) -> usize {
        self.steps.len()
    }

    pub fn is_reviewed(&self, step: &GuidedHunk) -> bool {
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
        self.loading.is_some() || self.fixing.is_some() || !self.asks.is_empty()
    }

    /// The notes that belong on `step`: those written on it, and those pinned
    /// to a line it still shows.
    pub fn notes_for(&self, step: &GuidedHunk) -> Vec<(usize, &GuidedNote)> {
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

    fn save(&self) -> Result<(), String> {
        crate::git::guided::save_progress(&self.source.progress_key(), &self.progress)
            .map_err(|e| format!("could not save review progress: {e:#}"))
    }
}

// ─── Opening and loading ─────────────────────────────────────────────────────

/// Walk the branch on screen against main, or on main itself, what has not
/// been committed yet.
pub fn open_branch(state: &mut AppState) {
    if state.repo_root.is_none() {
        state.set_status("no repository open to review", true);
        return;
    }
    let branch = state.branch.clone().unwrap_or_else(|| "HEAD".to_string());
    // On main the branch has nothing against itself, so what there is to
    // review is what has not been committed yet.
    let base = crate::preferences::base_branch();
    let source = if branch == base {
        Source::Changes { branch }
    } else {
        Source::Branch {
            branch,
            base_ref: base,
        }
    };
    if let Some(guided) = state.guided.as_ref()
        && guided.source.progress_key() == source.progress_key()
    {
        // Back to the walk already under way, rebuilt in case the code moved.
        state.modal = Modal::GuidedReview;
        reload(state);
        return;
    }
    let what = match &source {
        Source::Changes { .. } => "the uncommitted changes",
        _ => "the branch",
    };
    let mut guided = Guided::new(source);
    guided.loading = Some(load_local(&guided.source));
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state.set_status(format!("reading {what} for a guided review\u{2026}"), false);
}

/// Walk a pull request as GitHub has it.
pub fn open_pull_request(state: &mut AppState, pr: &crate::github::PullRequest) {
    if let Some(guided) = state.guided.as_ref()
        && matches!(&guided.source, Source::PullRequest { number, .. } if *number == pr.number)
    {
        state.modal = Modal::GuidedReview;
        return;
    }
    let local = state.branch.as_deref() == Some(pr.head.as_str());
    let mut guided = Guided::new(Source::PullRequest {
        number: pr.number,
        title: pr.title.clone(),
        head: pr.head.clone(),
        base: pr.base.clone(),
        commit: String::new(),
        local,
    });
    guided.loading = Some(load_pull_request(pr.clone(), local));
    guided.pull_request = Some(pr.clone());
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state.set_status(
        format!("reading #{} for a guided review\u{2026}", pr.number),
        false,
    );
}

/// Read the change again, keeping the place, the notes and every answer whose
/// hunk did not change.
pub fn reload(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.loading.is_some() {
        return;
    }
    guided.loading = Some(match (&guided.source, guided.pull_request.clone()) {
        (Source::PullRequest { local, .. }, Some(pr)) => load_pull_request(pr, *local),
        (source, _) => load_local(source),
    });
}

/// Read a walk over the checkout itself: the branch against main, or the
/// uncommitted changes.
fn load_local(source: &Source) -> Receiver<Result<Loaded, String>> {
    let changes = matches!(source, Source::Changes { .. });
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let diff = if changes {
            crate::git::uncommitted_diff()
        } else {
            crate::git::branch_diff_against_main()
        };
        let result = diff
            .map(|diff| {
                let steps = crate::git::guided::parse_hunks(&diff.diff);
                let (title, source) = if changes {
                    (
                        format!("Uncommitted changes on {}", diff.branch),
                        Source::Changes {
                            branch: diff.branch,
                        },
                    )
                } else {
                    (
                        format!("Branch {} against {}", diff.branch, diff.base_ref),
                        Source::Branch {
                            branch: diff.branch,
                            base_ref: diff.base_ref,
                        },
                    )
                };
                let overview = overview_text(&title, "", &diff.commits, &steps);
                Loaded {
                    source,
                    steps,
                    overview,
                }
            })
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

fn load_pull_request(
    pr: crate::github::PullRequest,
    local: bool,
) -> Receiver<Result<Loaded, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = (|| -> anyhow::Result<Loaded> {
            let diff = crate::github::pr_diff(pr.number)?;
            let commit = crate::github::pr_head(pr.number)?;
            let steps = crate::git::guided::parse_hunks(&diff);
            let overview = overview_text(
                &format!(
                    "Pull request #{} {} ({} into {})",
                    pr.number, pr.title, pr.head, pr.base
                ),
                &pr.body,
                &[],
                &steps,
            );
            Ok(Loaded {
                source: Source::PullRequest {
                    number: pr.number,
                    title: pr.title,
                    head: pr.head,
                    base: pr.base,
                    commit,
                    local,
                },
                steps,
                overview,
            })
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

/// What every prompt opens with: what the change says it is, and every file
/// it touches with how much.
fn overview_text(title: &str, body: &str, commits: &[String], steps: &[GuidedHunk]) -> String {
    let mut out = format!("{title}\n");
    if !body.trim().is_empty() {
        out.push_str("\nDescription:\n");
        out.push_str(body.trim());
        out.push('\n');
    }
    if !commits.is_empty() {
        out.push_str("\nCommits:\n");
        for commit in commits {
            out.push_str(&format!("- {commit}\n"));
        }
    }
    out.push_str("\nFiles changed:\n");
    let mut files: Vec<(&str, usize, usize, Option<&str>)> = Vec::new();
    for step in steps {
        match files.last_mut() {
            Some((path, added, removed, _)) if *path == step.path => {
                *added += step.added();
                *removed += step.removed();
            }
            _ => files.push((
                &step.path,
                step.added(),
                step.removed(),
                step.file_note.as_deref(),
            )),
        }
    }
    for (path, added, removed, note) in files {
        let note = note.map(|n| format!(" ({n})")).unwrap_or_default();
        out.push_str(&format!("- {path} +{added} -{removed}{note}\n"));
    }
    out
}

// ─── Polling ─────────────────────────────────────────────────────────────────

/// Take in whatever the background work produced since the last frame, and
/// start the commentary the reviewer is about to want.
pub fn poll(state: &mut AppState) {
    let ai = state.ai_assist;
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let mut notices: Vec<(String, bool)> = Vec::new();

    if let Some(rx) = guided.loading.as_ref() {
        match rx.try_recv() {
            Ok(Ok(loaded)) => {
                guided.loading = None;
                let first_load = guided.steps.is_empty() && guided.overview.is_empty();
                apply_loaded(guided, loaded, first_load);
                if first_load {
                    notices.push((
                        format!(
                            "guided review: {} step(s); \u{2192} next, c note, a ask, e edit, f fix",
                            guided.total()
                        ),
                        false,
                    ));
                } else {
                    notices.push(("guided review refreshed".to_string(), false));
                }
            }
            Ok(Err(message)) => {
                guided.loading = None;
                notices.push((format!("guided review: {message}"), true));
            }
            Err(TryRecvError::Disconnected) => guided.loading = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    let mut reload_after_fix = false;
    if let Some((rx, _)) = guided.fixing.as_ref() {
        match rx.try_recv() {
            Ok(result) => {
                guided.fixing = None;
                match result {
                    Ok(said) => {
                        notices.push((format!("claude: {said}"), false));
                        reload_after_fix = true;
                    }
                    Err(message) => notices.push((message, true)),
                }
            }
            Err(TryRecvError::Disconnected) => guided.fixing = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    drain_asks(guided);
    if ai {
        start_wanted_commentary(guided);
    }

    for (text, is_error) in notices {
        state.set_status(text, is_error);
    }
    if reload_after_fix {
        reload(state);
    }
}

fn apply_loaded(guided: &mut Guided, loaded: Loaded, first_load: bool) {
    let previous = guided.step().map(|step| (step.key(), step.path.clone()));
    // A fresh read keeps a pull request's commit, which only the load knows.
    guided.source = loaded.source;
    guided.steps = loaded.steps;
    guided.overview = loaded.overview;
    if let Some((key, path)) = previous {
        // The same hunk if it is still there, else the first one left in the
        // same file, else where the walk was.
        let index = guided
            .steps
            .iter()
            .position(|step| step.key() == key)
            .or_else(|| guided.steps.iter().position(|step| step.path == path));
        guided.current = match index {
            Some(index) => index + 1,
            None => guided.current.min(guided.steps.len()),
        };
    } else if first_load && !guided.progress.reviewed.is_empty() {
        // A walk picked up again opens where it stopped; a new one opens on
        // the overview.
        guided.current = first_unreviewed(guided).unwrap_or(OVERVIEW);
    }
    guided.current = guided.current.min(guided.steps.len());
    guided.cursor = guided.step().map(GuidedHunk::first_changed).unwrap_or(0);
    guided.side_scroll = 0;
}

fn first_unreviewed(guided: &Guided) -> Option<usize> {
    guided
        .steps
        .iter()
        .position(|step| !guided.is_reviewed(step))
        .map(|index| index + 1)
}

fn drain_asks(guided: &mut Guided) {
    let mut finished = Vec::new();
    for (index, ask) in guided.asks.iter().enumerate() {
        let entry = match &ask.target {
            Target::Commentary(key) => guided.commentary.entry(key.clone()).or_default(),
            Target::Answer(key, at) => match guided
                .exchanges
                .get_mut(key)
                .and_then(|exchanges| exchanges.get_mut(*at))
            {
                Some(exchange) => &mut exchange.answer,
                None => {
                    finished.push(index);
                    continue;
                }
            },
        };
        loop {
            match ask.rx.try_recv() {
                Ok(GenMsg::Thinking(_)) => entry.thinking = true,
                Ok(GenMsg::Output(chunk)) => {
                    entry.thinking = false;
                    entry.text.push_str(&chunk);
                }
                Ok(GenMsg::Reset) => entry.text.clear(),
                Ok(GenMsg::Done { text, stats }) => {
                    entry.text = text;
                    if stats.truncated {
                        entry.text.push('\n');
                        entry.text.push_str(crate::llm::TRUNCATED_NOTE);
                    }
                    entry.done = true;
                    entry.thinking = false;
                    finished.push(index);
                    break;
                }
                Ok(GenMsg::Error(message)) => {
                    entry.error = Some(message);
                    entry.done = true;
                    entry.thinking = false;
                    if let Target::Commentary(key) = &ask.target {
                        guided.failed.insert(key.clone());
                    }
                    finished.push(index);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    entry.done = true;
                    finished.push(index);
                    break;
                }
            }
        }
    }
    for index in finished.into_iter().rev() {
        guided.asks.remove(index);
    }
}

/// How many requests may run at once. The local server answers one request
/// per session at a time; Claude takes several.
fn concurrency() -> usize {
    match crate::llm::current_provider() {
        crate::llm::LlmProvider::Claude => LOOKAHEAD_CLAUDE,
        crate::llm::LlmProvider::Mtplx => LOOKAHEAD_LOCAL,
    }
}

/// Ask for the commentary on the step on screen and the few after it, nearest
/// first, as far as the provider allows at once.
fn start_wanted_commentary(guided: &mut Guided) {
    if guided.loading.is_some() || (guided.steps.is_empty() && guided.overview.is_empty()) {
        return;
    }
    let limit = concurrency();
    let running = guided
        .asks
        .iter()
        .filter(|ask| matches!(ask.target, Target::Commentary(_)))
        .count();
    if running >= limit {
        return;
    }
    let total = guided.total();
    let wanted: Vec<usize> = (guided.current..=guided.current + limit)
        .filter(|&i| i <= total)
        .collect();
    let mut slots = limit - running;
    for index in wanted {
        if slots == 0 {
            break;
        }
        let key = if index == OVERVIEW {
            OVERVIEW_KEY.to_string()
        } else {
            guided.steps[index - 1].key()
        };
        let asked = guided.commentary.contains_key(&key) || guided.failed.contains(&key);
        if asked {
            continue;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        if index == OVERVIEW {
            let context = guided.overview.clone();
            crate::git::spawn_pinned(move || crate::llm::stream_guided_overview(context, tx));
        } else {
            let context = step_context(guided, index);
            crate::git::spawn_pinned(move || crate::llm::stream_guided_step(context, tx));
        }
        guided.commentary.insert(key.clone(), Commentary::default());
        guided.asks.push(Ask {
            target: Target::Commentary(key),
            rx,
        });
        slots -= 1;
    }
}

/// Everything the model is shown about step `index`: the change's overview,
/// the hunk with every line numbered, and the source around it when the files
/// on disk are the ones being reviewed.
fn step_context(guided: &Guided, index: usize) -> String {
    let step = &guided.steps[index - 1];
    let mut out = String::new();
    out.push_str("Change overview:\n");
    out.push_str(&guided.overview);
    out.push_str(&format!(
        "\nHunk on screen: {} (hunk {} of {} in this file){}\n",
        step.path,
        step.hunk_index + 1,
        step.hunks_in_file,
        step.file_note
            .as_deref()
            .map(|note| format!(", {note}"))
            .unwrap_or_default()
    ));
    out.push_str(&step.header);
    out.push('\n');
    for line in &step.lines {
        let number = line
            .new
            .or(line.old)
            .map(|n| format!("L{n}"))
            .unwrap_or_default();
        out.push_str(&format!("{number:>6} {}\n", line.text));
    }
    if guided.source.editable()
        && let Some(source) = source_around(step)
    {
        out.push_str("\nThe file around this hunk, as it is now:\n");
        out.push_str(&source);
    }
    out
}

/// The working file's lines around the hunk, numbered.
fn source_around(step: &GuidedHunk) -> Option<String> {
    let first = step.lines.iter().find_map(|line| line.new)?;
    let last = step.lines.iter().rev().find_map(|line| line.new)?;
    let root = crate::git::repo_dir()?;
    let text = std::fs::read_to_string(root.join(&step.path)).ok()?;
    let from = first.saturating_sub(SOURCE_CONTEXT_LINES).max(1);
    let to = last + SOURCE_CONTEXT_LINES;
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if number < from {
            continue;
        }
        if number > to {
            break;
        }
        out.push_str(&format!("{:>6} {line}\n", format!("L{number}")));
    }
    Some(out)
}

// ─── Acting ──────────────────────────────────────────────────────────────────

/// Move to step `index`, marking the one being left as reviewed when moving
/// forward past it.
fn go_to(state: &mut AppState, index: usize, mark_left: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if mark_left && let Some(step) = guided.step() {
        let key = step.key();
        if !guided.progress.reviewed.contains(&key) {
            guided.progress.reviewed.push(key);
            if let Err(message) = guided.save() {
                state.set_status(message, true);
                return;
            }
        }
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    guided.current = index.min(guided.total());
    guided.cursor = guided.step().map(GuidedHunk::first_changed).unwrap_or(0);
    guided.side_scroll = 0;
    if guided.current == guided.total() && guided.reviewed_count() + 1 >= guided.total() {
        let noun = if guided.source.is_pull_request() {
            "s submits the review"
        } else {
            "s shows every note"
        };
        state.set_status(format!("last step \u{2014} {noun}"), false);
    }
}

pub(super) fn next_step(state: &mut AppState) {
    let Some((current, total)) = state.guided.as_ref().map(|g| (g.current, g.total())) else {
        return;
    };
    if current >= total {
        go_to(state, current, true);
        let all = state
            .guided
            .as_ref()
            .is_some_and(|g| g.reviewed_count() == g.total());
        if all {
            open_summary(state);
        }
        return;
    }
    go_to(state, current + 1, true);
}

pub(super) fn previous_step(state: &mut AppState) {
    let Some(current) = state.guided.as_ref().map(|g| g.current) else {
        return;
    };
    go_to(state, current.saturating_sub(1), false);
}

/// The first hunk of the next file, or of the previous one.
pub(super) fn step_file(state: &mut AppState, forward: bool) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    let file = guided.step().map(|step| step.file_index);
    let target = if forward {
        guided
            .steps
            .iter()
            .position(|step| Some(step.file_index) > file || file.is_none())
            .map(|index| index + 1)
    } else {
        let current_file = file.unwrap_or(0);
        let previous_file = guided
            .steps
            .iter()
            .rev()
            .find(|step| step.file_index < current_file)
            .map(|step| step.file_index);
        previous_file.and_then(|f| {
            guided
                .steps
                .iter()
                .position(|step| step.file_index == f)
                .map(|index| index + 1)
        })
    };
    match target {
        Some(index) => go_to(state, index, forward),
        None => state.set_status(
            if forward {
                "no more files"
            } else {
                "already at the first file"
            },
            false,
        ),
    }
}

pub(super) fn next_unreviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    let after = guided
        .steps
        .iter()
        .enumerate()
        .skip(guided.current)
        .chain(guided.steps.iter().enumerate().take(guided.current))
        .find(|(_, step)| !guided.is_reviewed(step))
        .map(|(index, _)| index + 1);
    match after {
        Some(index) => go_to(state, index, false),
        None => state.set_status("every step is reviewed", false),
    }
}

pub(super) fn toggle_reviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(key) = guided.step().map(GuidedHunk::key) else {
        return;
    };
    if let Some(at) = guided.progress.reviewed.iter().position(|k| *k == key) {
        guided.progress.reviewed.remove(at);
    } else {
        guided.progress.reviewed.push(key);
    }
    if let Err(message) = guided.save() {
        state.set_status(message, true);
    }
}

pub(super) fn move_cursor(state: &mut AppState, down: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(len) = guided.step().map(|step| step.lines.len()) else {
        guided.side_scroll = if down {
            guided.side_scroll.saturating_add(1)
        } else {
            guided.side_scroll.saturating_sub(1)
        };
        return;
    };
    guided.cursor = if down {
        (guided.cursor + 1).min(len.saturating_sub(1))
    } else {
        guided.cursor.saturating_sub(1)
    };
}

/// Save the note being written on the cursor line.
pub(super) fn commit_note(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Mode::Note { text, editing } = std::mem::take(&mut guided.mode) else {
        return;
    };
    let body = text.trim().to_string();
    match editing {
        Some(index) if body.is_empty() => {
            guided.progress.notes.remove(index);
        }
        Some(index) => {
            if let Some(note) = guided.progress.notes.get_mut(index) {
                note.body = body;
            }
        }
        None if body.is_empty() => return,
        None => {
            let Some(step) = guided.step() else {
                state.set_status("notes go on a hunk, not the overview", false);
                return;
            };
            let Some(line) = step.lines.get(guided.cursor) else {
                return;
            };
            let Some((number, side)) = line.anchor() else {
                state.set_status("move to a code line to pin the note to", false);
                return;
            };
            let note = GuidedNote {
                path: step.path.clone(),
                line: number,
                side,
                body,
                step: step.key(),
                quote: line.text.clone(),
            };
            guided.progress.notes.push(note);
        }
    }
    let count = guided.progress.notes.len();
    match guided.save() {
        Ok(()) => state.set_status(format!("{count} note(s) in this review"), false),
        Err(message) => state.set_status(message, true),
    }
}

pub(super) fn delete_note_at_cursor(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(index) = guided.note_at_cursor() else {
        state.set_status("no note on this line", false);
        return;
    };
    guided.progress.notes.remove(index);
    match guided.save() {
        Ok(()) => state.set_status("note deleted", false),
        Err(message) => state.set_status(message, true),
    }
}

/// Ask the model about the step on screen.
pub(super) fn ask_question(state: &mut AppState) {
    if !state.ai_assist {
        state.set_status(crate::panel::commit::AI_OFF_NOTICE, false);
        return;
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Mode::Question { text } = std::mem::take(&mut guided.mode) else {
        return;
    };
    let question = text.trim().to_string();
    if question.is_empty() {
        return;
    }
    let key = guided.step_key();
    let context = if guided.current == OVERVIEW {
        guided.overview.clone()
    } else {
        step_context(guided, guided.current)
    };
    let commentary = guided
        .commentary
        .get(&key)
        .filter(|c| c.done)
        .map(|c| c.text.clone())
        .unwrap_or_default();
    let exchanges = guided.exchanges.entry(key.clone()).or_default();
    exchanges.push(Exchange {
        question: question.clone(),
        answer: Commentary::default(),
    });
    let at = exchanges.len() - 1;
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        crate::llm::stream_guided_question(context, commentary, question, tx)
    });
    guided.asks.push(Ask {
        target: Target::Answer(key, at),
        rx,
    });
    guided.side_scroll = u16::MAX;
}

/// Throw away the commentary on the step on screen and ask for it again.
pub(super) fn regenerate(state: &mut AppState) {
    if !state.ai_assist {
        state.set_status(crate::panel::commit::AI_OFF_NOTICE, false);
        return;
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let key = guided.step_key();
    let running = guided
        .asks
        .iter()
        .any(|ask| ask.target == Target::Commentary(key.clone()));
    if running {
        state.set_status("already reading this step", false);
        return;
    }
    guided.commentary.remove(&key);
    guided.failed.remove(&key);
}

/// Have Claude Code make the change the reviewer described, in the checkout.
pub(super) fn start_fix(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Mode::Fix { text, all_notes } = std::mem::take(&mut guided.mode) else {
        return;
    };
    if guided.fixing.is_some() {
        state.set_status("claude is still applying the last fix", false);
        return;
    }
    let Some(root) = state.repo_root.clone() else {
        state.set_status("no checkout to change", true);
        return;
    };
    let instruction = text.trim().to_string();
    let (context, what) = if all_notes {
        (notes_markdown(guided), "every note".to_string())
    } else {
        if instruction.is_empty() {
            state.set_status("say what to change; Esc cancels", false);
            return;
        }
        let Some(step) = guided.step() else {
            return;
        };
        let line = step
            .lines
            .get(guided.cursor)
            .and_then(|line| line.new.or(line.old))
            .map(|n| format!(" (cursor on L{n})"))
            .unwrap_or_default();
        (
            format!(
                "{}\nThe reviewer is looking at {}{line}.",
                step_context(guided, guided.current),
                step.path
            ),
            step.path.clone(),
        )
    };
    let instruction = if all_notes && instruction.is_empty() {
        "Address every review note below.".to_string()
    } else {
        instruction
    };
    let prompt = crate::llm::build_guided_fix_prompt(&context, &instruction);
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = crate::llm::run_claude_fix(std::path::Path::new(&root), &prompt)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    guided.fixing = Some((rx, what.clone()));
    state.set_status(format!("claude is changing {what}\u{2026}"), false);
}

/// Open the file at the cursor line in the terminal editor; the walk is read
/// again once the editor exits.
pub(super) fn edit_at_cursor(state: &mut AppState, in_ide: bool) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    if !guided.source.editable() {
        state.set_status(
            "this pull request is not checked out here \u{2014} check it out (Enter in GitHub) to edit it",
            false,
        );
        return;
    }
    let Some(step) = guided.step() else {
        state.set_status("pick a hunk to edit", false);
        return;
    };
    if step.file_note.as_deref() == Some("deleted file") {
        state.set_status("this file is deleted on the branch", false);
        return;
    }
    let line = step.editor_line(guided.cursor);
    let path = step.path.clone();
    state.pending_action = Some(if in_ide {
        PendingAction::OpenFile(path)
    } else {
        PendingAction::EditFile { path, line }
    });
}

pub(super) fn open_summary(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    guided.mode = Mode::Summary {
        event: 0,
        body: String::new(),
    };
    guided.side_scroll = 0;
}

/// Submit the notes as one review of the pull request.
pub(super) fn submit(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Source::PullRequest { number, commit, .. } = &guided.source else {
        return;
    };
    let Mode::Summary { event, body } = std::mem::take(&mut guided.mode) else {
        return;
    };
    let event = crate::github::ReviewEvent::ALL[event % crate::github::ReviewEvent::ALL.len()];
    let shown: HashSet<(String, usize, Side)> = guided
        .steps
        .iter()
        .flat_map(|step| {
            step.lines
                .iter()
                .filter_map(|line| line.anchor())
                .map(|(n, side)| (step.path.clone(), n, side))
        })
        .collect();
    let (comments, stale): (Vec<_>, Vec<_>) = guided
        .progress
        .notes
        .iter()
        .partition(|note| shown.contains(&(note.path.clone(), note.line, note.side)));
    if comments.is_empty() && body.trim().is_empty() && event != crate::github::ReviewEvent::Approve
    {
        guided.mode = Mode::Summary {
            event: crate::github::ReviewEvent::ALL
                .iter()
                .position(|e| *e == event)
                .unwrap_or(0),
            body,
        };
        state.set_status("nothing to submit: write a note or a summary first", false);
        return;
    }
    let mut body = body.trim().to_string();
    if !stale.is_empty() {
        // A note whose line the diff no longer shows cannot be pinned, so it
        // goes in the body rather than being dropped.
        body.push_str("\n\n");
        for note in &stale {
            body.push_str(&format!("- `{}:{}` {}\n", note.path, note.line, note.body));
        }
    }
    let comments = comments
        .into_iter()
        .map(|note| crate::github::LineComment {
            path: note.path.clone(),
            line: note.line,
            side: note.side.github(),
            body: note.body.clone(),
        })
        .collect();
    state.pending_action = Some(PendingAction::GitHub(
        crate::state::GitHubAction::SubmitReview {
            number: *number,
            commit: commit.clone(),
            event,
            body: body.trim().to_string(),
            comments,
        },
    ));
}

/// The review went to GitHub: its notes are there now, and sending them again
/// would post every comment twice. A failed submit keeps them to try again.
pub fn after_submit(state: &mut AppState, succeeded: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !succeeded || !guided.source.is_pull_request() {
        return;
    }
    guided.progress.notes.clear();
    if let Err(message) = guided.save() {
        state.set_status(message, true);
    }
}

/// Every note as Markdown, file by file, for the clipboard or a fix request.
pub(super) fn notes_markdown(guided: &Guided) -> String {
    let mut notes: Vec<&GuidedNote> = guided.progress.notes.iter().collect();
    notes.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    let mut out = format!("Review notes: {}\n", guided.source.title());
    let mut path = "";
    for note in notes {
        if note.path != path {
            path = &note.path;
            out.push_str(&format!("\n## {path}\n"));
        }
        let side = if note.side == Side::Left {
            " (removed line)"
        } else {
            ""
        };
        out.push_str(&format!("- L{}{side}: {}\n", note.line, note.body));
        let quote = note.quote.trim();
        if !quote.is_empty() {
            out.push_str(&format!("  > `{quote}`\n"));
        }
    }
    out
}

/// Leave the walk. Its progress is already on disk; the commentary stays in
/// memory, so coming back the same session is instant.
pub(super) fn leave(state: &mut AppState) {
    let back = match state.guided.as_ref().map(|g| &g.source) {
        Some(Source::PullRequest { .. }) => Modal::GitHub,
        _ => Modal::None,
    };
    if let Some(guided) = state.guided.as_mut() {
        guided.mode = Mode::Browse;
    }
    state.modal = back;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commentary(text: &str) -> Commentary {
        Commentary {
            text: text.to_string(),
            done: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_verdict_is_read_off_the_last_line() {
        assert_eq!(
            commentary("Adds a cache.\nVerdict: issue").verdict(),
            Some(Verdict::Issue)
        );
        assert_eq!(
            commentary("Fine.\n\nVerdict: **ok**\n").verdict(),
            Some(Verdict::Ok)
        );
        assert_eq!(commentary("No verdict here").verdict(), None);
    }

    #[test]
    fn an_unfinished_answer_has_no_verdict_yet() {
        let mut c = commentary("Verdict: ok");
        c.done = false;

        assert_eq!(c.verdict(), None);
    }

    #[test]
    fn the_overview_lists_each_file_once_with_its_totals() {
        let steps = crate::git::guided::parse_hunks(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,2 @@\n x\n+y\n@@ -9,1 +10,1 @@\n-p\n+q\n",
        );
        let text = overview_text("Branch b", "", &["abc fix it".into()], &steps);

        assert!(text.contains("- abc fix it"));
        assert_eq!(text.matches("a.rs").count(), 1, "{text}");
        assert!(text.contains("+2 -1"), "{text}");
    }
}
