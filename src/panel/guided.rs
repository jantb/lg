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
use std::sync::mpsc::Receiver;

use crate::git::guided::{GuidedHunk, GuidedNote, GuidedProgress, Side};
use crate::panel::text_input::TextInput;
use crate::state::{AppState, GenMsg, Modal, PendingAction, Poll, poll_once, stopped_unexpectedly};
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

mod draw;
mod keys;
mod work;

pub use draw::render;
pub use keys::{handle_key, handle_paste};

/// The wheel over the step list moves between steps, over the hunk moves its
/// cursor, and over the commentary scrolls it (or the notes, in the summary).
/// A click on a step in the list goes to it, and the divider beside the list
/// drags to resize it. None of it marks a step reviewed: that stays with the
/// keys that say so.
pub fn handle_mouse(state: &mut AppState, area: ratatui::layout::Rect, m: &MouseEvent) {
    use crate::panel::pointer;
    let Some((list, side)) = state
        .guided
        .as_deref()
        .and_then(|guided| draw::pane_areas(guided, area, state.guided_list_width))
    else {
        return;
    };
    if resize_list(state, list, m) {
        return;
    }
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.typing() && !matches!(guided.mode, Mode::Summary { .. }) {
        return;
    }
    let current = guided.current;
    let total = guided.total();
    let browsing = matches!(guided.mode, Mode::Browse);
    match pointer::wheel(m) {
        Some(down) if pointer::inside(list, m.column, m.row) && browsing => {
            let next = if down {
                (current + 1).min(total)
            } else {
                current.saturating_sub(1)
            };
            go_to(state, next, false);
        }
        Some(down) if pointer::inside(side, m.column, m.row) => {
            // The hunk scrolls with its cursor, as j and k move it.
            let over_hunk = browsing
                && guided.step().is_some_and(|step| {
                    let (parts, _) = draw::step_areas(guided, step, side);
                    pointer::inside(parts[0], m.column, m.row)
                });
            if over_hunk {
                for _ in 0..3 {
                    move_cursor(state, down);
                }
            } else {
                keys::scroll_side(state, if down { 3 } else { -3 });
            }
        }
        Some(_) => {}
        None if pointer::left_click(m) && browsing => {
            let (rows, first) = draw::step_rows(guided, list.height);
            if let Some(at) = pointer::list_row(list, first, rows.len(), m.column, m.row)
                && let Some(step) = rows[at]
            {
                go_to(state, step, false);
            }
        }
        None => {}
    }
}

/// The divider right of the step list, dragged, sets how wide the list is.
/// Returns whether the event went to the drag.
fn resize_list(state: &mut AppState, list: ratatui::layout::Rect, m: &MouseEvent) -> bool {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let divider = list.x.saturating_add(list.width);
            let rows = list.y..list.y.saturating_add(list.height);
            state.guided_list_drag_active = m.column == divider && rows.contains(&m.row);
            state.guided_list_drag_active
        }
        MouseEventKind::Drag(MouseButton::Left) if state.guided_list_drag_active => {
            state.guided_list_width = Some(m.column.saturating_sub(list.x));
            true
        }
        MouseEventKind::Up(MouseButton::Left) if state.guided_list_drag_active => {
            state.guided_list_drag_active = false;
            true
        }
        _ => false,
    }
}

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
        text: TextInput,
        editing: Option<usize>,
    },
    /// Asking the model about the step on screen.
    Question { text: TextInput },
    /// Telling Claude what to change.
    Fix { text: TextInput, all_notes: bool },
    /// Every note, and for a pull request the review they are submitted as.
    Summary { event: usize, body: TextInput },
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
    /// The note `x` was pressed on once; pressing it again deletes it.
    pub(super) delete_armed: Option<usize>,
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
            delete_armed: None,
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
    open_local(state, source);
}

/// Walk the checkout as `source` has it, or go back to that walk if it is the
/// one already under way.
fn open_local(state: &mut AppState, source: Source) {
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
    guided.loading = Some(work::load_local(&guided.source));
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state.set_status(format!("reading {what} for a guided review\u{2026}"), false);
}

/// Switch the walk between the whole branch against main and only what has
/// not been committed yet. Each keeps its own progress and notes, so switching
/// back resumes where that walk stopped.
pub(super) fn switch_scope(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.fixing.is_some() {
        state.set_status(
            "claude is still changing the code; switch once it is done",
            true,
        );
        return;
    }
    let base = crate::preferences::base_branch();
    let source = match &guided.source {
        Source::Branch { branch, .. } => Source::Changes {
            branch: branch.clone(),
        },
        Source::Changes { branch } if *branch == base => {
            state.set_status(
                format!("{branch} is the base branch; there is no branch against it to review"),
                true,
            );
            return;
        }
        Source::Changes { branch } => Source::Branch {
            branch: branch.clone(),
            base_ref: base,
        },
        Source::PullRequest { .. } => {
            state.set_status("a pull request is walked as GitHub has it", true);
            return;
        }
    };
    open_local(state, source);
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
    guided.loading = Some(work::load_pull_request(pr.clone(), local));
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
        (Source::PullRequest { local, .. }, Some(pr)) => work::load_pull_request(pr, *local),
        (source, _) => work::load_local(source),
    });
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
        match poll_once(rx) {
            Poll::Message(Ok(loaded)) => {
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
            Poll::Message(Err(message)) => {
                guided.loading = None;
                notices.push((format!("guided review: {message}"), true));
            }
            Poll::Disconnected => {
                guided.loading = None;
                notices.push((
                    format!(
                        "guided review: {}",
                        stopped_unexpectedly("reading the change")
                    ),
                    true,
                ));
            }
            Poll::Pending => {}
        }
    }

    let mut reload_after_fix = false;
    if let Some((rx, _)) = guided.fixing.as_ref() {
        match poll_once(rx) {
            Poll::Message(result) => {
                guided.fixing = None;
                match result {
                    Ok(said) => {
                        notices.push((format!("claude: {said}"), false));
                        reload_after_fix = true;
                    }
                    Err(message) => notices.push((message, true)),
                }
            }
            Poll::Disconnected => {
                guided.fixing = None;
                notices.push((stopped_unexpectedly("the claude fix"), true));
            }
            Poll::Pending => {}
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
            match poll_once(&ask.rx) {
                Poll::Message(GenMsg::Thinking(_)) => entry.thinking = true,
                Poll::Message(GenMsg::Output(chunk)) => {
                    entry.thinking = false;
                    entry.text.push_str(&chunk);
                }
                Poll::Message(GenMsg::Reset) => entry.text.clear(),
                Poll::Message(GenMsg::Done { text, stats }) => {
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
                Poll::Message(GenMsg::Error(message)) => {
                    entry.error = Some(message);
                    entry.done = true;
                    entry.thinking = false;
                    if let Target::Commentary(key) = &ask.target {
                        guided.failed.insert(key.clone());
                    }
                    finished.push(index);
                    break;
                }
                Poll::Pending => break,
                // The worker died before the answer was done.
                Poll::Disconnected => {
                    entry.error = Some(stopped_unexpectedly("the model request"));
                    entry.done = true;
                    entry.thinking = false;
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
        let rx = if index == OVERVIEW {
            work::ask_overview(guided.overview.clone())
        } else {
            work::ask_step(step_context(guided, index))
        };
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
    let armed = guided.delete_armed.take();
    let Some(index) = guided.note_at_cursor() else {
        state.set_status("no note on this line", false);
        return;
    };
    // A note is the one thing the walk cannot give back, so the first press
    // only asks.
    if armed != Some(index) {
        guided.delete_armed = Some(index);
        state.set_status("x again deletes the note on this line", false);
        return;
    }
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
    let rx = work::ask_question(context, commentary, question);
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
    guided.fixing = Some((work::run_fix(root, prompt), what.clone()));
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
        body: TextInput::default(),
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

    /// A read whose worker died must end the loading state and say why,
    /// rather than leave the walk waiting on it.
    #[test]
    fn a_read_of_the_change_that_dies_is_reported() {
        let mut guided = Guided::new(Source::Changes {
            branch: "feature".into(),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        guided.loading = Some(rx);
        drop(tx);
        let mut state = AppState::new();
        state.guided = Some(Box::new(guided));

        poll(&mut state);

        assert!(state.guided.as_ref().unwrap().loading.is_none());
        let status = state.status.expect("the failure is reported");
        assert!(status.is_error);
        assert!(
            status.text.contains("stopped unexpectedly"),
            "{}",
            status.text
        );
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

    const AREA: ratatui::layout::Rect = ratatui::layout::Rect::new(0, 0, 120, 40);

    /// A walk over a branch, on its one hunk, long enough to need scrolling.
    fn walk_on_a_long_hunk() -> AppState {
        let mut diff =
            String::from("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,40 +1,40 @@\n");
        for n in 0..40 {
            diff.push_str(&format!("-old {n}\n+new {n}\n"));
        }
        let mut guided = Guided::new(Source::Branch {
            branch: "guided-test-branch".into(),
            base_ref: "main".into(),
        });
        guided.steps = crate::git::guided::parse_hunks(&diff);
        guided.current = 1;
        let mut state = AppState::new();
        state.guided = Some(Box::new(guided));
        state.modal = Modal::GuidedReview;
        state
    }

    fn panes(state: &AppState) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
        draw::pane_areas(
            state.guided.as_deref().unwrap(),
            AREA,
            state.guided_list_width,
        )
        .unwrap()
    }

    fn mouse(state: &mut AppState, kind: MouseEventKind, column: u16, row: u16) {
        let event = MouseEvent {
            kind,
            column,
            row,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        };
        handle_mouse(state, AREA, &event);
    }

    #[test]
    fn the_step_list_is_as_wide_as_its_divider_is_dragged() {
        let mut state = walk_on_a_long_hunk();
        let (list, _) = panes(&state);
        let divider = list.x + list.width;
        let row = list.y + 2;

        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            divider,
            row,
        );
        mouse(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            divider + 10,
            row,
        );
        mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            divider + 10,
            row,
        );
        let (wider, side) = panes(&state);
        assert_eq!(wider.x + wider.width, divider + 10);
        assert!(side.x > divider + 10, "the hunk moves over for it");

        // A drag that did not start on the divider leaves the list be.
        mouse(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            divider + 20,
            row,
        );
        assert_eq!(panes(&state).0, wider);
    }

    /// The verdict sits in the list's first column, where a list dragged
    /// narrow cannot cut it off.
    #[test]
    fn a_steps_verdict_shows_at_the_left_edge_of_a_narrow_list() {
        let mut state = walk_on_a_long_hunk();
        state.guided_list_width = Some(12);
        let guided = state.guided.as_mut().unwrap();
        let key = guided.steps[0].key();
        guided
            .commentary
            .insert(key, commentary("Drops a check.\nVerdict: issue"));
        let (list, _) = panes(&state);

        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(&state, AREA, frame)).unwrap();
        let buf = terminal.backend().buffer();

        assert!(
            (list.y..list.y + list.height).any(|y| buf[(list.x, y)].symbol() == "\u{25cf}"),
            "no verdict at the list's left edge"
        );
    }

    #[test]
    fn the_wheel_over_the_hunk_scrolls_the_hunk_and_over_the_commentary_the_commentary() {
        let mut state = walk_on_a_long_hunk();
        let (_, side) = panes(&state);
        let guided = state.guided.as_deref().unwrap();
        let (parts, _) = draw::step_areas(guided, guided.step().unwrap(), side);
        let (hunk, commentary) = (parts[0], parts[1]);

        mouse(
            &mut state,
            MouseEventKind::ScrollDown,
            hunk.x + 5,
            hunk.y + 3,
        );
        let cursor = state.guided.as_ref().unwrap().cursor;
        assert!(cursor > 0, "the hunk's cursor moves down it");

        mouse(
            &mut state,
            MouseEventKind::ScrollDown,
            commentary.x + 5,
            commentary.y + 1,
        );
        let guided = state.guided.as_ref().unwrap();
        assert_eq!(guided.cursor, cursor, "the hunk stays where it was");
        assert!(guided.side_scroll > 0, "the commentary scrolls");
    }

    #[test]
    fn b_switches_between_the_whole_branch_and_what_is_uncommitted() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE);
        let mut state = walk_on_a_long_hunk();

        handle_key(&mut state, b).unwrap();
        assert_eq!(
            state.guided.as_ref().unwrap().source,
            Source::Changes {
                branch: "guided-test-branch".into()
            }
        );

        handle_key(&mut state, b).unwrap();
        assert_eq!(
            state.guided.as_ref().unwrap().source,
            Source::Branch {
                branch: "guided-test-branch".into(),
                base_ref: crate::preferences::base_branch(),
            }
        );
    }

    /// On the base branch there is no branch against it to walk, so the walk
    /// over its uncommitted changes stays put and says why.
    #[test]
    fn the_base_branch_has_only_its_uncommitted_changes_to_walk() {
        let mut state = AppState::new();
        let branch = crate::preferences::base_branch();
        state.guided = Some(Box::new(Guided::new(Source::Changes {
            branch: branch.clone(),
        })));
        state.modal = Modal::GuidedReview;

        switch_scope(&mut state);

        assert_eq!(
            state.guided.as_ref().unwrap().source,
            Source::Changes { branch }
        );
        assert!(state.status.is_some_and(|status| status.is_error));
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
