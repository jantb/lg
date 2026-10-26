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

use crate::git::guided::{GuidedHunk, GuidedNote, GuidedProgress, Side, TickedFinding};
use crate::panel::text_input::TextInput;
use crate::state::{
    AppState, DiffViewMode, GenMsg, Modal, PendingAction, Poll, poll_once, stopped_unexpectedly,
};
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

mod draw;
mod findings;
mod keys;
mod layout;
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
    if chat_mouse(state, side, m) {
        return;
    }
    if select_text(state, side, m) {
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
                    let (parts, _) = draw::step_areas(guided, step, side, state.diff_view_mode);
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
        None if pointer::left_click(m) && browsing && pointer::inside(side, m.column, m.row) => {
            // Ticking waits for the release: a drag selects text instead.
            let area = draw::selectable_areas(guided, side, state.diff_view_mode)
                .into_iter()
                .find(|area| pointer::inside(*area, m.column, m.row));
            state.selection =
                area.and_then(|area| crate::ui::TextSelection::within(area, m.column, m.row));
        }
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

fn follow_file_in_chat(guided: &mut Guided, sessions: &mut crate::session::Sessions) {
    let Some(session) = guided.chat.and_then(|id| sessions.get_mut(id)) else {
        return;
    };
    let Some(path) = guided.step().map(|step| step.path.clone()) else {
        return;
    };
    if guided.chat_file.as_deref() == Some(path.as_str()) {
        return;
    }
    let message = crate::llm::build_guided_chat_next_file(&guided.chat_brief(&path));
    let bracketed = session.bracketed_paste();
    session.send(&crate::term::encode_paste(&message, bracketed));
    session.send(b"\r");
    guided.chat_file = Some(path);
}

fn report_redo_to_chat(guided: &mut Guided, sessions: &mut crate::session::Sessions) {
    let settled: Vec<String> = guided
        .chat_update
        .iter()
        .filter(|path| redo_settled(guided, path))
        .cloned()
        .collect();
    guided.chat_update.retain(|path| !settled.contains(path));
    let Some(session) = guided.chat.and_then(|id| sessions.get_mut(id)) else {
        return;
    };
    for path in settled {
        let message = crate::llm::build_guided_chat_redo(&guided.chat_brief(&path));
        let bracketed = session.bracketed_paste();
        session.send(&crate::term::encode_paste(&message, bracketed));
        session.send(b"\r");
    }
}

/// A press and release that never moved is a click, which ticks the finding under it.
fn select_text(state: &mut AppState, side: ratatui::layout::Rect, m: &MouseEvent) -> bool {
    match m.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            match state
                .selection
                .as_mut()
                .filter(|selection| selection.dragging)
            {
                Some(selection) => {
                    selection.extend(m.column, m.row);
                    true
                }
                None => false,
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let Some(pressed) = state.selection.take() else {
                return false;
            };
            state.selection = pressed.release();
            let clicked = state.selection.is_none();
            let finding = state
                .guided
                .as_deref()
                .filter(|guided| clicked && guided.mode == Mode::Browse)
                .and_then(|guided| {
                    draw::finding_at(guided, side, state.diff_view_mode, m.column, m.row)
                });
            if let Some(finding) = finding {
                toggle_ticks(state, vec![finding]);
            }
            true
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            state.selection = None;
            false
        }
        _ => false,
    }
}

/// A click elsewhere while the chat has the keyboard takes it back.
fn chat_mouse(state: &mut AppState, side: ratatui::layout::Rect, m: &MouseEvent) -> bool {
    use crate::panel::pointer;
    let Some(id) = chat_session(state) else {
        return false;
    };
    let mode = state.diff_view_mode;
    let screen = state
        .guided
        .as_deref()
        .and_then(|guided| draw::chat_screen_area(guided, side, mode));
    let over = screen.filter(|area| pointer::inside(*area, m.column, m.row));
    let Some(guided) = state.guided.as_mut() else {
        return false;
    };
    match (pointer::wheel(m), over) {
        (Some(down), Some(area)) => {
            if let Some(session) = state.sessions.get_mut(id)
                && !session.send_wheel(!down, m.column - area.x, m.row - area.y)
            {
                session.scroll(!down, 3);
            }
            true
        }
        (None, Some(_)) if pointer::left_click(m) => {
            guided.mode = Mode::Chat;
            true
        }
        (None, None) if pointer::left_click(m) && guided.mode == Mode::Chat => {
            guided.mode = Mode::Browse;
            false
        }
        _ => false,
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
    pub steps: Vec<GuidedHunk>,
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
    pub steps: Vec<GuidedHunk>,
    pub overview: String,
    brief: String,
    /// Pinned to the walk's first brief, so the cached step prompt never changes.
    shared_brief: Option<String>,
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
    /// The model's last read of each file `r` sent back, for the next read to say what is fixed.
    earlier: HashMap<String, String>,
    /// The note `x` was pressed on once; pressing it again deletes it.
    pub(super) delete_armed: Option<usize>,
    pub chat: Option<crate::session::SessionId>,
    pub chat_file: Option<String>,
    /// Files whose new read the chat is told about once it is in.
    chat_update: Vec<String>,
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
            failed: HashSet::new(),
            earlier: HashMap::new(),
            delete_armed: None,
            chat: None,
            chat_file: None,
            chat_update: Vec::new(),
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

    fn visit_order(&self, mode: DiffViewMode) -> Vec<usize> {
        self.step()
            .map(|step| layout::visit_order(&layout::rows(step, mode)))
            .unwrap_or_default()
    }

    fn flagged_lines(&self, mode: DiffViewMode) -> Vec<usize> {
        let flagged: HashSet<usize> = self
            .placed_findings()
            .into_iter()
            .filter(|(_, finding)| !self.is_ticked(&self.ticket(finding)))
            .map(|(at, _)| at)
            .collect();
        self.visit_order(mode)
            .into_iter()
            .filter(|at| flagged.contains(at))
            .collect()
    }

    pub(super) fn findings_at_cursor(&self) -> String {
        self.placed_findings()
            .into_iter()
            .filter(|(at, _)| *at == self.cursor)
            .map(|(_, finding)| finding.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn placed_findings(&self) -> Vec<(usize, findings::Finding)> {
        let Some(step) = self.step() else {
            return Vec::new();
        };
        let Some(commentary) = self.commentary.get(&step.key()) else {
            return Vec::new();
        };
        let reading = findings::read(&commentary.text);
        findings::place(step, &reading.findings)
            .0
            .into_iter()
            .map(|(at, finding)| (at, finding.clone()))
            .collect()
    }

    fn ticket(&self, finding: &findings::Finding) -> TickedFinding {
        ticket_in(
            self.step()
                .map(|step| step.path.as_str())
                .unwrap_or_default(),
            finding,
        )
    }

    pub fn chat_brief(&self, path: &str) -> String {
        let mut out = format!("Open in {path}:\n");
        for step in self.steps.iter().filter(|step| step.path == path) {
            let Some(commentary) = self.commentary.get(&step.key()).filter(|c| c.done) else {
                continue;
            };
            let reading = findings::read(&commentary.text);
            for fixed in &reading.fixed {
                out.push_str(&format!("- fixed: {fixed}\n"));
            }
            let open = reading
                .findings
                .into_iter()
                .filter(|finding| !self.is_ticked(&ticket_in(path, finding)));
            for finding in open {
                let word = match finding.severity {
                    findings::Severity::Issue => "issue",
                    findings::Severity::Nit => "nit",
                };
                out.push_str(&format!("- L{} {word}: {}\n", finding.line, finding.text));
            }
        }
        for note in self.progress.notes.iter().filter(|note| note.path == path) {
            out.push_str(&format!("- L{} note: {}\n", note.line, note.body));
        }
        out
    }

    /// The worst finding not yet ticked off; an answer that named none keeps its own verdict.
    pub fn verdict(&self, step: &GuidedHunk) -> Option<Verdict> {
        let commentary = self.commentary.get(&step.key())?;
        let given = commentary.verdict()?;
        let findings = findings::read(&commentary.text).findings;
        if findings.is_empty() {
            return Some(given);
        }
        Some(
            findings
                .iter()
                .filter(|finding| !self.is_ticked(&ticket_in(&step.path, finding)))
                .map(|finding| match finding.severity {
                    findings::Severity::Issue => Verdict::Issue,
                    findings::Severity::Nit => Verdict::Nit,
                })
                .max()
                .unwrap_or(Verdict::Ok),
        )
    }

    /// (issues, nits) not yet ticked off.
    pub fn open_in_file(&self, path: &str) -> (usize, usize) {
        self.steps
            .iter()
            .filter(|step| step.path == path)
            .filter_map(|step| self.commentary.get(&step.key()).filter(|c| c.done))
            .flat_map(|commentary| findings::read(&commentary.text).findings)
            .filter(|finding| !self.is_ticked(&ticket_in(path, finding)))
            .fold((0, 0), |(issues, nits), finding| match finding.severity {
                findings::Severity::Issue => (issues + 1, nits),
                findings::Severity::Nit => (issues, nits + 1),
            })
    }

    pub fn is_ticked(&self, finding: &TickedFinding) -> bool {
        self.progress.ticked.contains(finding)
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

/// Open the pull request under review on GitHub. A walk over local changes has
/// no page to open.
pub(super) fn open_in_browser(state: &mut AppState) {
    let url = state
        .guided
        .as_deref()
        .and_then(|guided| guided.pull_request.as_ref())
        .map(|pr| pr.url.clone())
        .filter(|url| !url.is_empty());
    match url {
        Some(url) => {
            state.pending_action = Some(PendingAction::GitHub(
                crate::state::GitHubAction::OpenInBrowser { url },
            ))
        }
        None => state.set_status("only a pull request has a page to open", false),
    }
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

fn overview_of(title: &str, body: &str, commits: &[String], steps: &[GuidedHunk]) -> Overview {
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
    let (mut text, mut brief) = (out.clone(), out);
    for (path, added, removed, note) in files {
        let note = note.map(|n| format!(" ({n})")).unwrap_or_default();
        text.push_str(&format!("- {path} +{added} -{removed}{note}\n"));
        brief.push_str(&format!("- {path}{note}\n"));
    }
    Overview { text, brief }
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
    if guided
        .chat
        .is_some_and(|id| state.sessions.get(id).is_none())
    {
        guided.chat = None;
        guided.chat_update.clear();
        if guided.mode == Mode::Chat {
            guided.mode = Mode::Browse;
        }
        notices.push(("the review chat ended".to_string(), false));
    }
    follow_file_in_chat(guided, &mut state.sessions);
    report_redo_to_chat(guided, &mut state.sessions);

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
    guided.overview = loaded.overview.text;
    guided.brief = loaded.overview.brief;
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

/// Most wanted first. Another file is read only once the walk reaches it, or while the chat is owed its redo.
fn steps_to_read(guided: &Guided) -> Vec<usize> {
    let path_of = |i: usize| guided.steps[i - 1].path.as_str();
    let here = guided.step().map(|step| step.path.as_str());
    let total = guided.total();
    let mut seen = HashSet::new();
    std::iter::once(guided.current)
        .chain((guided.current + 1..=total).filter(|&i| Some(path_of(i)) == here))
        .chain((1..guided.current.min(total)).filter(|&i| Some(path_of(i)) == here))
        .chain((1..=total).filter(|&i| guided.chat_update.iter().any(|path| path == path_of(i))))
        .filter(|&i| seen.insert(i))
        .collect()
}

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
    let wanted = steps_to_read(guided);
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
            // The walk's first brief, so the cached prompt stays the same.
            let shared = guided
                .shared_brief
                .get_or_insert_with(|| guided.brief.clone())
                .clone();
            work::ask_step(shared, hunk_context(guided, index))
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
    format!(
        "Change overview:\n{}{}",
        guided.overview,
        hunk_context(guided, index)
    )
}

fn hunk_context(guided: &Guided, index: usize) -> String {
    let step = &guided.steps[index - 1];
    let mut out = String::new();
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
    if guided
        .shared_brief
        .as_ref()
        .is_some_and(|shared| *shared != guided.brief)
    {
        out.push_str(
            "\nThe change has moved on since the overview you were given; it now reads:\n",
        );
        out.push_str(&guided.brief);
    }
    if let Some(earlier) = guided.earlier.get(&step.path) {
        out.push_str(
            "\nYour earlier read of this file, before the reviewer changed it. Give each finding \
             the code now fixes as a `Fixed:` line, and repeat only what still stands:\n",
        );
        out.push_str(earlier);
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
    let mode = state.diff_view_mode;
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.step().is_none() {
        guided.side_scroll = if down {
            guided.side_scroll.saturating_add(1)
        } else {
            guided.side_scroll.saturating_sub(1)
        };
        return;
    }
    let order = guided.visit_order(mode);
    let here = order
        .iter()
        .position(|&at| at == guided.cursor)
        .unwrap_or(0);
    let next = if down {
        here + 1
    } else {
        here.saturating_sub(1)
    };
    if let Some(&at) = order.get(next) {
        guided.cursor = at;
    }
}

/// Until the model has read the step there is nothing to jump to, so it moves a line.
pub(super) fn next_finding(state: &mut AppState) {
    let (ai, mode) = (state.ai_assist, state.diff_view_mode);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.step().is_none() || guided.still_reading(ai) {
        move_cursor(state, true);
        return;
    }
    let (order, flagged) = (guided.visit_order(mode), guided.flagged_lines(mode));
    let place = |at: usize| order.iter().position(|&o| o == at);
    let here = place(guided.cursor);
    if let Some(at) = flagged.into_iter().find(|&at| place(at) > here) {
        guided.cursor = at;
        return;
    }
    let left = guided.current;
    next_step(state);
    land_on_finding(state, left, true);
}

pub(super) fn previous_finding(state: &mut AppState) {
    let (ai, mode) = (state.ai_assist, state.diff_view_mode);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.step().is_none() || guided.still_reading(ai) {
        move_cursor(state, false);
        return;
    }
    let (order, flagged) = (guided.visit_order(mode), guided.flagged_lines(mode));
    let place = |at: usize| order.iter().position(|&o| o == at);
    let here = place(guided.cursor);
    if let Some(at) = flagged.into_iter().rev().find(|&at| place(at) < here) {
        guided.cursor = at;
        return;
    }
    let left = guided.current;
    previous_step(state);
    land_on_finding(state, left, false);
}

fn land_on_finding(state: &mut AppState, left: usize, first: bool) {
    let mode = state.diff_view_mode;
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.current == left || !matches!(guided.mode, Mode::Browse) {
        return;
    }
    let flagged = guided.flagged_lines(mode);
    if let Some(&at) = if first {
        flagged.first()
    } else {
        flagged.last()
    } {
        guided.cursor = at;
    }
}

pub(super) fn tick_at_cursor(state: &mut AppState) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    let here: Vec<TickedFinding> = guided
        .placed_findings()
        .into_iter()
        .filter(|(at, _)| *at == guided.cursor)
        .map(|(_, finding)| guided.ticket(&finding))
        .collect();
    if here.is_empty() {
        state.set_status("nothing flagged on this line", false);
        return;
    }
    toggle_ticks(state, here);
}

fn toggle_ticks(state: &mut AppState, findings: Vec<TickedFinding>) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let ticked = &mut guided.progress.ticked;
    if findings.iter().all(|finding| ticked.contains(finding)) {
        ticked.retain(|finding| !findings.contains(finding));
    } else {
        let new: Vec<TickedFinding> = findings
            .into_iter()
            .filter(|f| !ticked.contains(f))
            .collect();
        ticked.extend(new);
    }
    if let Err(message) = guided.save() {
        state.set_status(message, true);
    }
}

pub(super) fn open_chat(state: &mut AppState) {
    let running = chat_session(state);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !guided.source.editable() {
        state.set_status(
            "this pull request is not checked out here \u{2014} check it out to fix it in a chat",
            false,
        );
        return;
    }
    if guided.step().is_none() {
        state.set_status("the chat is about a file: \u{2192} to a hunk first", false);
        return;
    }
    if running.is_some() {
        guided.mode = Mode::Chat;
        state.set_status("chat \u{2014} Ctrl-] back to the review", false);
        return;
    }
    state.pending_action = Some(PendingAction::GuidedChat);
}

pub(super) fn chat_session(state: &AppState) -> Option<crate::session::SessionId> {
    state
        .guided
        .as_ref()?
        .chat
        .filter(|id| state.sessions.get(*id).is_some())
}

pub fn chat_on_screen(state: &AppState) -> Option<crate::session::SessionId> {
    (state.modal == Modal::GuidedReview)
        .then(|| chat_session(state))
        .flatten()
}

/// Then every key, Ctrl-C and Esc included, goes to Claude.
pub fn chat_has_keyboard(state: &AppState) -> bool {
    state.modal == Modal::GuidedReview
        && state
            .guided
            .as_ref()
            .is_some_and(|guided| guided.mode == Mode::Chat)
}

pub fn fit_chat(state: &mut AppState, area: ratatui::layout::Rect) {
    if state.modal != Modal::GuidedReview {
        return;
    }
    let Some(id) = chat_session(state) else {
        return;
    };
    let mode = state.diff_view_mode;
    let Some(screen) = state.guided.as_deref().and_then(|guided| {
        let (_, side) = draw::pane_areas(guided, area, state.guided_list_width)?;
        draw::chat_screen_area(guided, side, mode)
    }) else {
        return;
    };
    if let Some(session) = state.sessions.get_mut(id) {
        session.resize(screen.height, screen.width);
    }
}

fn ticket_in(path: &str, finding: &findings::Finding) -> TickedFinding {
    TickedFinding {
        path: path.to_string(),
        line: finding.line,
        text: finding.text.clone(),
    }
}

/// Shared with the diff pane.
pub(super) fn toggle_layout(state: &mut AppState) {
    state.diff_view_mode = state.diff_view_mode.toggled();
    state.set_status(format!("showing {}", state.diff_view_mode.label()), false);
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

/// Hands the model its earlier read of the file, so it can say what the edit fixed.
pub(super) fn review_again(state: &mut AppState) {
    if !state.ai_assist {
        state.set_status(crate::panel::commit::AI_OFF_NOTICE, false);
        return;
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(path) = guided.step().map(|step| step.path.clone()) else {
        forget(guided, &[OVERVIEW_KEY.to_string()]);
        reload(state);
        return;
    };
    let keys: Vec<String> = guided
        .steps
        .iter()
        .filter(|step| step.path == path)
        .map(GuidedHunk::key)
        .collect();
    let said: Vec<String> = guided
        .steps
        .iter()
        .filter(|step| step.path == path)
        .filter_map(|step| {
            let commentary = guided.commentary.get(&step.key()).filter(|c| c.done)?;
            Some(format!("{}\n{}", step.header, commentary.text.trim()))
        })
        .collect();
    if !said.is_empty() {
        guided.earlier.insert(path.clone(), said.join("\n\n"));
    }
    forget(guided, &keys);
    if guided.chat.is_some() && !guided.chat_update.contains(&path) {
        guided.chat_update.push(path.clone());
    }
    state.set_status(format!("reviewing {path} again\u{2026}"), false);
    reload(state);
}

/// A file the change no longer touches has nothing left to wait for.
fn redo_settled(guided: &Guided, path: &str) -> bool {
    guided.loading.is_none()
        && guided
            .steps
            .iter()
            .filter(|step| step.path == path)
            .all(|step| guided.commentary.get(&step.key()).is_some_and(|c| c.done))
}

fn forget(guided: &mut Guided, keys: &[String]) {
    guided.asks.retain(|ask| match &ask.target {
        Target::Commentary(key) => !keys.contains(key),
        Target::Answer(..) => true,
    });
    for key in keys {
        guided.commentary.remove(key);
        guided.failed.remove(key);
    }
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

    fn press(state: &mut AppState, code: ratatui::crossterm::event::KeyCode) {
        let key = ratatui::crossterm::event::KeyEvent::new(
            code,
            ratatui::crossterm::event::KeyModifiers::NONE,
        );
        handle_key(state, key).unwrap();
    }

    fn opened_in_browser(state: &AppState) -> Option<&str> {
        match &state.pending_action {
            Some(PendingAction::GitHub(crate::state::GitHubAction::OpenInBrowser { url })) => {
                Some(url)
            }
            _ => None,
        }
    }

    /// A pull request being walked can be looked at on GitHub too.
    #[test]
    fn a_pull_request_under_review_opens_in_the_browser() {
        use crate::github::{Checks, Mergeable, PrState, PullRequest, ReviewDecision};
        let pr = PullRequest {
            number: 7,
            title: "Add a cache".into(),
            author: "alice".into(),
            head: "cache".into(),
            head_owner: None,
            base: "main".into(),
            state: PrState::Open,
            draft: false,
            review: ReviewDecision::ReviewRequired,
            reviews: Vec::new(),
            checks: Checks::default(),
            mergeable: Mergeable::Yes,
            url: "https://github.com/acme/app/pull/7".into(),
            body: String::new(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            updated_at: String::new(),
        };
        let mut guided = Guided::new(Source::PullRequest {
            number: pr.number,
            title: pr.title.clone(),
            head: pr.head.clone(),
            base: pr.base.clone(),
            commit: String::new(),
            local: false,
        });
        guided.pull_request = Some(pr);
        let mut state = AppState::new();
        state.guided = Some(Box::new(guided));
        state.modal = Modal::GuidedReview;

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('O'));

        assert_eq!(
            opened_in_browser(&state),
            Some("https://github.com/acme/app/pull/7")
        );
    }

    /// A walk over local changes has no page, so nothing is opened and the
    /// reason is given instead.
    #[test]
    fn a_walk_over_local_changes_has_nothing_to_open() {
        let mut state = AppState::new();
        state.guided = Some(Box::new(Guided::new(Source::Changes {
            branch: "feature".into(),
        })));
        state.modal = Modal::GuidedReview;

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('O'));

        assert_eq!(opened_in_browser(&state), None);
        assert!(state.status.is_some());
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
    fn the_verdict_is_read_off_the_last_line_once_the_answer_is_done() {
        assert_eq!(
            commentary("Adds a cache.\nVerdict: issue").verdict(),
            Some(Verdict::Issue)
        );
        assert_eq!(
            commentary("Fine.\n\nVerdict: **ok**\n").verdict(),
            Some(Verdict::Ok)
        );
        assert_eq!(commentary("No verdict here").verdict(), None);

        let mut unfinished = commentary("Verdict: ok");
        unfinished.done = false;
        assert_eq!(unfinished.verdict(), None);
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
        let (parts, _) =
            draw::step_areas(guided, guided.step().unwrap(), side, state.diff_view_mode);
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
        let Overview { text, brief } = overview_of("Branch b", "", &["abc fix it".into()], &steps);

        assert!(text.contains("- abc fix it"));
        assert_eq!(text.matches("a.rs").count(), 1, "{text}");
        assert!(text.contains("+2 -1"), "{text}");
        assert!(brief.contains("- a.rs\n"), "{brief}");
        assert!(!brief.contains("+2 -1"), "{brief}");
    }

    /// Both hunks start reviewed, so walking past them saves nothing to disk.
    fn walk_with_findings() -> AppState {
        let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
                    @@ -1,3 +1,3 @@\n keep\n-old one\n+new one\n tail\n\
                    @@ -20,3 +20,3 @@\n keep\n-old two\n+new two\n keep\n";
        let mut guided = Guided::new(Source::Branch {
            branch: "guided-test-findings".into(),
            base_ref: "main".into(),
        });
        guided.steps = crate::git::guided::parse_hunks(diff);
        guided.progress.reviewed = guided.steps.iter().map(GuidedHunk::key).collect();
        guided.commentary.insert(
            guided.steps[0].key(),
            commentary(
                "Summary: Renames a line.\n\
                 L2 issue: -1 stands in for missing and is used as an index.\n\
                 L3 nit: tail could say what it ends.\n\
                 Verdict: issue",
            ),
        );
        guided.commentary.insert(
            guided.steps[1].key(),
            commentary("Summary: Same rename.\nVerdict: ok"),
        );
        guided.current = 1;
        guided.cursor = guided.steps[0].first_changed();
        let mut state = AppState::new();
        state.guided = Some(Box::new(guided));
        state.modal = Modal::GuidedReview;
        state
    }

    fn cursor_line(state: &AppState) -> (usize, String) {
        let guided = state.guided.as_deref().unwrap();
        let line = &guided.step().unwrap().lines[guided.cursor];
        (guided.current, line.text.clone())
    }

    fn screen(state: &AppState) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(state, AREA, frame)).unwrap();
        let buf = terminal.backend().buffer();
        (0..AREA.height)
            .map(|y| (0..AREA.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn the_arrows_walk_the_flagged_lines_across_hunks() {
        use ratatui::crossterm::event::KeyCode::{Char, Down, Esc, Up};
        let mut state = walk_with_findings();

        press(&mut state, Down);
        assert_eq!(cursor_line(&state), (1, "+new one".into()));

        press(&mut state, Char('c'));
        let Mode::Note { text, .. } = &state.guided.as_ref().unwrap().mode else {
            panic!("c opens a note");
        };
        assert!(text.contains("-1 stands in for missing"), "{}", &**text);
        press(&mut state, Esc);

        press(&mut state, Down);
        assert_eq!(cursor_line(&state), (1, " tail".into()));
        press(&mut state, Down);
        assert_eq!(cursor_line(&state).0, 2);
        press(&mut state, Up);
        assert_eq!(cursor_line(&state), (1, " tail".into()));
    }

    #[test]
    fn down_moves_a_line_while_the_model_is_still_reading() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        let key = guided.steps[0].key();
        guided.commentary.get_mut(&key).unwrap().done = false;
        let start = guided.cursor;

        press(&mut state, ratatui::crossterm::event::KeyCode::Down);

        let guided = state.guided.as_ref().unwrap();
        assert_eq!((guided.current, guided.cursor), (1, start + 1));
    }

    #[test]
    fn the_chat_starts_on_what_is_open_in_the_file_on_screen() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        guided.progress.ticked.push(TickedFinding {
            path: "a.rs".into(),
            line: 3,
            text: "tail could say what it ends.".into(),
        });
        let other = crate::git::guided::parse_hunks(
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n",
        )
        .remove(0);
        guided.commentary.insert(
            other.key(),
            commentary("L1 issue: in another file\nVerdict: issue"),
        );
        guided.steps.push(other);

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('C'));

        assert_eq!(state.pending_action, Some(PendingAction::GuidedChat));
        let brief = state.guided.as_ref().unwrap().chat_brief("a.rs");
        assert!(
            brief.contains("L2 issue: -1 stands in for missing"),
            "{brief}"
        );
        assert!(!brief.contains("tail could say"), "{brief}");
        assert!(!brief.contains("in another file"), "{brief}");

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('Y'));
        let Some(PendingAction::CopyToClipboard { text, .. }) = &state.pending_action else {
            panic!("Y copies");
        };
        assert_eq!(*text, brief);
    }

    #[test]
    fn the_model_reads_ahead_only_in_the_file_on_screen() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        guided.steps.extend(crate::git::guided::parse_hunks(
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n",
        ));

        guided.current = 2;
        assert_eq!(
            steps_to_read(guided),
            vec![2, 1],
            "the whole file on screen"
        );
        guided.current = 3;
        assert_eq!(steps_to_read(guided), vec![3], "not a file the walk left");
        guided.current = OVERVIEW;
        assert_eq!(steps_to_read(guided), vec![OVERVIEW]);

        guided.chat_update.push("a.rs".into());
        guided.current = 3;
        assert_eq!(steps_to_read(guided), vec![3, 1, 2]);
    }

    #[test]
    fn ticking_findings_off_settles_the_hunk_and_its_file() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        let first = guided.steps[0].clone();
        assert_eq!(guided.verdict(&first), Some(Verdict::Issue));
        assert_eq!(guided.open_in_file("a.rs"), (1, 1));

        guided.progress.ticked.push(TickedFinding {
            path: "a.rs".into(),
            line: 2,
            text: "-1 stands in for missing and is used as an index.".into(),
        });
        assert_eq!(guided.verdict(&first), Some(Verdict::Nit));
        assert_eq!(guided.open_in_file("a.rs"), (0, 1));

        guided.progress.ticked.push(TickedFinding {
            path: "a.rs".into(),
            line: 3,
            text: "tail could say what it ends.".into(),
        });
        assert_eq!(guided.verdict(&first), Some(Verdict::Ok));
        assert_eq!(guided.open_in_file("a.rs"), (0, 0));
    }

    #[test]
    fn r_reviews_the_file_again_with_what_was_said_before() {
        let mut state = walk_with_findings();
        state.ai_assist = true;
        // Already loading, so the test starts no read of its own.
        let (_reading, rx) = std::sync::mpsc::channel();
        state.guided.as_mut().unwrap().loading = Some(rx);

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('r'));

        let guided = state.guided.as_ref().unwrap();
        assert!(!guided.commentary.contains_key(&guided.steps[0].key()));
        assert!(hunk_context(guided, 1).contains("-1 stands in for missing"));

        let guided = state.guided.as_mut().unwrap();
        assert!(!redo_settled(guided, "a.rs"));
        guided.loading = None;
        let key = guided.steps[0].key();
        guided.commentary.insert(
            key,
            commentary("Summary: Same.\nFixed: the index sentinel.\nVerdict: ok"),
        );
        assert!(!redo_settled(guided, "a.rs"), "one of the file's two hunks");
        let key = guided.steps[1].key();
        guided
            .commentary
            .insert(key, commentary("Summary: Same.\nVerdict: ok"));
        assert!(redo_settled(guided, "a.rs"));
        assert!(
            redo_settled(guided, "gone.rs"),
            "a file the change no longer has"
        );
        assert!(
            guided
                .chat_brief("a.rs")
                .contains("fixed: the index sentinel.")
        );
    }

    #[test]
    fn the_hunk_follows_the_diff_layout_with_findings_under_their_line() {
        let mut state = walk_with_findings();
        let hunk = |state: &AppState| {
            let (_, side) = panes(state);
            let guided = state.guided.as_deref().unwrap();
            let (parts, _) =
                draw::step_areas(guided, guided.step().unwrap(), side, state.diff_view_mode);
            parts[0].y..parts[0].y + parts[0].height
        };
        let row_of =
            |rows: &[String], text: &str| rows.iter().position(|row| row.contains(text)).unwrap();

        assert_eq!(state.diff_view_mode, DiffViewMode::SideBySide);
        let rows = screen(&state);
        assert_eq!(row_of(&rows, "old one"), row_of(&rows, "new one"));
        let finding = row_of(&rows, "-1 stands in for missing");
        assert!(finding > row_of(&rows, "new one"));
        assert!(hunk(&state).contains(&(finding as u16)));
        let column = |row: &str, text: &str| row[..row.find(text).unwrap()].chars().count();
        let old = row_of(&rows, "old one");
        assert!(
            column(&rows[finding], "-1 stands in")
                > column(&rows[old], "old one") + "old one".len(),
            "side by side, the finding sits right of the old code, under the new"
        );

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('v'));

        assert_eq!(state.diff_view_mode, DiffViewMode::Unified);
        let rows = screen(&state);
        assert!(row_of(&rows, "new one") > row_of(&rows, "old one"));
        let finding = row_of(&rows, "-1 stands in for missing");
        assert!(finding > row_of(&rows, "new one"));
        assert!(hunk(&state).contains(&(finding as u16)));
    }

    #[test]
    fn the_walk_says_whether_it_covers_the_branch_or_only_uncommitted_code() {
        let shows = |source: Source| {
            let mut state = AppState::new();
            state.guided = Some(Box::new(Guided::new(source)));
            state.modal = Modal::GuidedReview;
            let rows = screen(&state);
            (
                rows.iter().any(|row| row.contains("BRANCH")),
                rows.iter().any(|row| row.contains("UNCOMMITTED")),
            )
        };

        assert_eq!(
            shows(Source::Branch {
                branch: "guided-test-scope".into(),
                base_ref: "main".into(),
            }),
            (true, false)
        );
        assert_eq!(
            shows(Source::Changes {
                branch: "guided-test-scope".into(),
            }),
            (false, true)
        );
    }
}
