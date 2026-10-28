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

use crate::git::guided::{GuidedFile, GuidedNote, GuidedProgress, Side, TickedFinding, WalkMode};
use crate::panel::text_input::TextInput;
use crate::state::{
    AppState, DiffViewMode, GenMsg, Modal, PendingAction, Poll, poll_once, stopped_unexpectedly,
};
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

mod draw;
mod findings;
mod keys;
mod layout;
mod plan;
mod tree;
mod work;

pub use draw::render;
pub use keys::{handle_key, handle_paste};

/// The wheel over the step list scrolls it, over the diff moves its cursor
/// (or on the final step scrolls the whole changeset), and over the
/// commentary scrolls it (or the notes, in the summary). Only a click on a
/// step in the list goes to it, which is what has the model read it. The
/// divider beside the list drags to resize it, the scope badge switches
/// scope, and the reset button under the list resets the review on a second
/// click. None of it marks a step reviewed: that stays with the keys that
/// say so.
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
    if pointer::left_click(m) && button_click(state, area, list, m) {
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
    let browsing = matches!(guided.mode, Mode::Browse);
    let steps = draw::steps_area(list);
    match pointer::wheel(m) {
        Some(down) if pointer::inside(list, m.column, m.row) && browsing => {
            let (rows, first) = draw::row_targets(guided, steps.height);
            let last = rows.len().saturating_sub(usize::from(steps.height));
            let first = if down {
                (first + 3).min(last)
            } else {
                first.saturating_sub(3)
            };
            if let Some(guided) = state.guided.as_mut() {
                guided.list_scroll = Some(first);
            }
        }
        Some(down) if pointer::inside(side, m.column, m.row) => {
            let mode = state.diff_view_mode;
            let over_diff = browsing
                && draw::diff_area(guided, side, mode)
                    .is_some_and(|diff| pointer::inside(diff, m.column, m.row));
            match (over_diff, guided.on_final()) {
                (true, true) => scroll_final(state, if down { 3 } else { -3 }),
                // The file's diff scrolls with its cursor, as j and k move it.
                (true, false) => {
                    for _ in 0..3 {
                        move_cursor(state, down);
                    }
                }
                (false, _) => keys::scroll_side(state, if down { 3 } else { -3 }),
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
            let (rows, first) = draw::row_targets(guided, steps.height);
            let target = pointer::list_row(steps, first, rows.len(), m.column, m.row)
                .and_then(|at| rows[at].clone());
            // The list stays where it was scrolled, under the mouse.
            let scrolled = Some(first);
            match target {
                Some(draw::RowTarget::Step(step)) => go_to(state, step, false),
                Some(draw::RowTarget::Finding(open)) => go_to_finding(state, &open),
                Some(draw::RowTarget::Concern(index))
                    if draw::on_concern_button(guided, steps, index, m.column) =>
                {
                    read_concern(state, index)
                }
                Some(draw::RowTarget::Concern(_)) | None => {}
            }
            if let Some(guided) = state.guided.as_mut() {
                guided.list_scroll = scrolled;
            }
        }
        None => {}
    }
}

/// The scope badge and the reset button. Any other click disarms the reset.
/// Returns whether the click went to one of them.
fn button_click(
    state: &mut AppState,
    area: ratatui::layout::Rect,
    list: ratatui::layout::Rect,
    m: &MouseEvent,
) -> bool {
    use crate::panel::pointer;
    let Some(guided) = state.guided.as_deref_mut() else {
        return false;
    };
    if guided.mode != Mode::Browse {
        return false;
    }
    if pointer::inside(draw::reset_button_area(list), m.column, m.row) {
        reset_review(state);
        return true;
    }
    guided.reset_armed = false;
    let on = |badge: Option<ratatui::layout::Rect>| {
        badge.is_some_and(|badge| pointer::inside(badge, m.column, m.row))
    };
    if on(draw::scope_badge_area(guided, area)) {
        switch_scope(state);
        return true;
    }
    if on(draw::walk_badge_area(guided, area)) {
        switch_walk(state);
        return true;
    }
    false
}

fn note_file_change_for_chat(guided: &mut Guided) {
    let Some(path) = guided.step().map(|step| step.path.clone()) else {
        return;
    };
    if guided.chat.is_none() || guided.chat_file.as_deref() == Some(path.as_str()) {
        return;
    }
    guided.chat_file = Some(path.clone());
    guided.owe_chat(path, ChatNews::FileRead);
}

/// Each piece of news waits until lg has read the whole file it is about.
fn tell_chat_what_is_read(guided: &mut Guided, sessions: &mut crate::session::Sessions) {
    let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut guided.chat_owed)
        .into_iter()
        .partition(|(path, _)| guided.has_read(path));
    guided.chat_owed = waiting;
    let Some(session) = guided.chat.and_then(|id| sessions.get_mut(id)) else {
        return;
    };
    for (path, news) in ready {
        let brief = guided.chat_brief(&path);
        let message = match news {
            ChatNews::FileRead => crate::llm::build_guided_chat_file_read(&brief),
            ChatNews::Redone => crate::llm::build_guided_chat_redo(&brief),
        };
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
                .and_then(|_| draw::finding_at(state, side, m.column, m.row));
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

/// A click in the chat gives it the keyboard and starts a selection kept to
/// its screen; a click elsewhere while the chat has the keyboard takes it back.
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
        (None, Some(area)) if pointer::left_click(m) => {
            guided.mode = Mode::Chat;
            // A drag from here selects the chat's own text, and no further.
            state.selection = crate::ui::TextSelection::within(area, m.column, m.row);
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

/// The step before the first file: what the change is for.
pub const OVERVIEW: usize = 0;
const OVERVIEW_KEY: &str = "overview";
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
            Self::Branch { .. } | Self::Unpushed { .. } => true,
            Self::PullRequest { local, .. } => *local,
        }
    }

    fn progress_key(&self) -> String {
        match self {
            Self::Branch { branch, .. } => format!("branch-{branch}"),
            Self::Unpushed { branch, .. } => format!("unpushed-{branch}"),
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
    /// How far the whole changeset is scrolled on the final step.
    pub final_scroll: u16,
    /// How far it could scroll when it was last drawn, as [`Guided::side_max`].
    pub final_max: std::cell::Cell<u16>,
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
            final_scroll: 0,
            final_max: std::cell::Cell::new(0),
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

    /// Every finding not yet ticked off in what has been read, the final
    /// review's included: issues, then nits, then questions, each in the
    /// walk's order and down the file.
    pub fn open_findings(&self) -> Vec<OpenFinding> {
        self.findings_in_order()
            .into_iter()
            .filter(|(_, ticked)| !ticked)
            .map(|(open, _)| open)
            .collect()
    }

    /// Every finding of what has been read in [`Guided::open_findings`]'
    /// order, the ticked ones kept in their place and said to be ticked, so
    /// ticking the one on screen does not lose the walk its place.
    fn findings_in_order(&self) -> Vec<(OpenFinding, bool)> {
        let mut out = Vec::new();
        let mut push = |step: usize, finding: findings::Finding, whole: bool| {
            let file = &self.steps[step - 1];
            let ticked = self.is_ticked(&ticket_in(&file.path, &finding));
            let at = findings::place(file, std::slice::from_ref(&finding))
                .0
                .first()
                .map(|(at, _)| *at);
            out.push((
                OpenFinding {
                    step,
                    at,
                    finding,
                    whole,
                },
                ticked,
            ));
        };
        for (index, step) in self.steps.iter().enumerate() {
            let Some(commentary) = self
                .commentary
                .get(&step.key())
                .filter(|c| c.done || c.checking)
            else {
                continue;
            };
            for finding in findings::read(&commentary.text).findings {
                push(index + 1, finding, false);
            }
        }
        if let Some(commentary) = self.commentary.get(&self.final_key()).filter(|c| c.done) {
            for (path, finding) in findings::read(&commentary.text).cited {
                if let Some(index) = self.steps.iter().position(|step| step.path == path) {
                    push(index + 1, finding, true);
                }
            }
        }
        let order = self.walk_order();
        let place = |step: usize| {
            order
                .iter()
                .position(|&at| at == step)
                .unwrap_or(order.len())
        };
        out.sort_by_key(|(open, _)| {
            (
                std::cmp::Reverse(open.finding.severity),
                place(open.step),
                open.finding.line,
            )
        });
        out
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
                let word = finding.severity.word();
                out.push_str(&format!("- L{} {word}: {}\n", finding.line, finding.text));
            }
        }
        for note in self.progress.notes.iter().filter(|note| note.path == path) {
            out.push_str(&format!("- L{} note: {}\n", note.line, note.body));
        }
        out
    }

    /// The worst finding not yet ticked off; an answer that named none keeps its own verdict.
    pub fn verdict(&self, step: &GuidedFile) -> Option<Verdict> {
        let commentary = self.commentary.get(&step.key())?;
        let given = commentary.verdict()?;
        let findings = findings::read(&commentary.text).findings;
        let cited = findings.iter().map(|finding| (step.path.as_str(), finding));
        Some(self.settled(given, cited))
    }

    /// The final review's verdict, settled by ticks as a file's is.
    pub fn final_verdict(&self) -> Option<Verdict> {
        let commentary = self.commentary.get(&self.final_key())?;
        let given = commentary.verdict()?;
        let cited = findings::read(&commentary.text).cited;
        Some(self.settled(
            given,
            cited.iter().map(|(path, finding)| (path.as_str(), finding)),
        ))
    }

    fn settled<'a>(
        &self,
        given: Verdict,
        findings: impl Iterator<Item = (&'a str, &'a findings::Finding)>,
    ) -> Verdict {
        let mut findings = findings.peekable();
        if findings.peek().is_none() {
            return given;
        }
        findings
            .filter(|(path, finding)| !self.is_ticked(&ticket_in(path, finding)))
            .map(|(_, finding)| match finding.severity {
                findings::Severity::Issue => Verdict::Issue,
                findings::Severity::Nit => Verdict::Nit,
                findings::Severity::Question => Verdict::Ok,
            })
            .max()
            .unwrap_or(Verdict::Ok)
    }

    /// How many findings of `severity` in the file are not yet ticked off.
    pub fn open_in_file(&self, path: &str, severity: findings::Severity) -> usize {
        self.steps
            .iter()
            .filter(|step| step.path == path)
            .filter_map(|step| self.commentary.get(&step.key()).filter(|c| c.done))
            .flat_map(|commentary| findings::read(&commentary.text).findings)
            .filter(|finding| finding.severity == severity)
            .filter(|finding| !self.is_ticked(&ticket_in(path, finding)))
            .count()
    }

    /// A file the change no longer touches has nothing left to wait for.
    pub fn has_read(&self, path: &str) -> bool {
        self.loading.is_none()
            && self
                .steps
                .iter()
                .filter(|step| step.path == path)
                .all(|step| self.commentary.get(&step.key()).is_some_and(|c| c.done))
    }

    pub fn owe_chat(&mut self, path: String, news: ChatNews) {
        match self.chat_owed.iter_mut().find(|(owed, _)| *owed == path) {
            Some((_, owed)) => *owed = (*owed).max(news),
            None => self.chat_owed.push((path, news)),
        }
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

/// Walk the branch on screen against main, or on main itself, what has not
/// been pushed yet.
pub fn open_branch(state: &mut AppState) {
    if state.repo_root.is_none() {
        state.set_status("no repository open to review", true);
        return;
    }
    let branch = state.branch.clone().unwrap_or_else(|| "HEAD".to_string());
    // On main the branch has nothing against itself, so what there is to
    // review is what has not been pushed yet.
    let base = crate::preferences::base_branch();
    let source = if branch == base {
        Source::Unpushed {
            branch,
            base_ref: String::new(),
        }
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
        Source::Unpushed { .. } => "what is not pushed",
        _ => "the branch",
    };
    let mut guided = Guided::new(source);
    guided.loading = Some(work::load_local(&guided.source));
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state.set_status(format!("reading {what} for a guided review\u{2026}"), false);
}

/// Switch the walk between the whole branch against main and only what has
/// not been pushed yet. Each keeps its own progress and notes, so switching
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
        Source::Branch { branch, .. } => Source::Unpushed {
            branch: branch.clone(),
            base_ref: String::new(),
        },
        Source::Unpushed { branch, .. } if *branch == base => {
            state.set_status(
                format!("{branch} is the base branch; there is no branch against it to review"),
                true,
            );
            return;
        }
        Source::Unpushed { branch, .. } => Source::Branch {
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
    // The number alone could be another repository's pull request: the inbox
    // opens reviews across the workspace.
    if let Some(guided) = state.guided.as_ref()
        && matches!(&guided.source, Source::PullRequest { number, .. } if *number == pr.number)
        && guided
            .pull_request
            .as_ref()
            .is_none_or(|open| open.url == pr.url)
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

fn overview_of(title: &str, body: &str, commits: &[String], steps: &[GuidedFile]) -> Overview {
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
    let root = state.repo_root.clone();
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
                            "guided review: {} file(s); \u{2192} next, c note, a ask, e edit, f fix",
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

    let answered = drain_asks(guided);
    // Without a checkout there is no code to settle questions in.
    if let Some(root) = &root {
        guided.check_queue.extend(answered);
        while guided.checks.len() < CONCURRENT_CHECKS && !guided.check_queue.is_empty() {
            let key = guided.check_queue.remove(0);
            start_question_check(guided, root, &key, &mut state.sessions);
        }
    }
    drain_checks(guided, &mut notices);
    if ai {
        start_wanted_commentary(guided);
    }
    if guided
        .chat
        .is_some_and(|id| state.sessions.get(id).is_none())
    {
        guided.chat = None;
        guided.chat_owed.clear();
        if guided.mode == Mode::Chat {
            guided.mode = Mode::Browse;
        }
        notices.push(("the review chat ended".to_string(), false));
    }
    note_file_change_for_chat(guided);
    tell_chat_what_is_read(guided, &mut state.sessions);

    for (text, is_error) in notices {
        state.set_status(text, is_error);
    }
    if reload_after_fix {
        reload(state);
    }
}

fn apply_loaded(guided: &mut Guided, loaded: Loaded, first_load: bool) {
    let previous = guided.step().map(|step| (step.key(), step.path.clone()));
    let was_final = guided.on_final();
    // A fresh read keeps a pull request's commit, which only the load knows.
    guided.source = loaded.source;
    guided.steps = loaded.steps;
    guided.overview = loaded.overview.text;
    guided.brief = loaded.overview.brief;
    if was_final {
        guided.current = guided.final_step();
    } else if let Some((key, path)) = previous {
        // The same change if it is still there, else the same file, else
        // where the walk was.
        let index = guided
            .steps
            .iter()
            .position(|step| step.key() == key)
            .or_else(|| guided.steps.iter().position(|step| step.path == path));
        guided.current = match index {
            Some(index) => index + 1,
            None => guided.current.min(guided.total()),
        };
    } else if first_load && !guided.progress.reviewed.is_empty() {
        // A walk picked up again opens where it stopped; a new one opens on
        // the overview.
        guided.current = first_unreviewed(guided).unwrap_or(OVERVIEW);
    }
    guided.current = guided.current.min(guided.final_step());
    guided.cursor = guided.step().map(GuidedFile::first_changed).unwrap_or(0);
    guided.side_scroll = 0;
}

fn first_unreviewed(guided: &Guided) -> Option<usize> {
    guided
        .walk_order()
        .into_iter()
        .find(|&step| !guided.is_reviewed(&guided.steps[step - 1]))
}

/// The keys of the steps whose commentary came in whole.
fn drain_asks(guided: &mut Guided) -> Vec<String> {
    let mut finished = Vec::new();
    let mut answered = Vec::new();
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
                    if let Target::Commentary(key) = &ask.target {
                        answered.push(key.clone());
                    }
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
    answered
}

/// Send the questions step `key`'s commentary raised to Claude, to be settled
/// in the checkout, when the files there are the ones being reviewed.
fn start_question_check(
    guided: &mut Guided,
    root: &str,
    key: &str,
    sessions: &mut crate::session::Sessions,
) {
    if !guided.source.editable() {
        return;
    }
    let Some(index) = guided.steps.iter().position(|step| step.key() == key) else {
        return;
    };
    let Some(commentary) = guided.commentary.get(key) else {
        return;
    };
    let (without, questions) = findings::split_questions(&commentary.text);
    if questions.is_empty() {
        return;
    }
    let step = &guided.steps[index];
    let chat = guided
        .chat
        .and_then(|id| sessions.get_mut(id))
        .and_then(|chat| {
            let file = crate::hooks::fresh_session_file(
                std::path::Path::new(root),
                &answer_file_name(key),
            )
            .ok()?;
            Some((chat, file))
        });
    let answer = match chat {
        Some((chat, file)) => {
            let message =
                crate::llm::build_guided_chat_questions(&step.path, &questions.join("\n"), &file);
            let bracketed = chat.bracketed_paste();
            chat.send(&crate::term::encode_paste(&message, bracketed));
            chat.send(b"\r");
            CheckAnswer::Chat(file)
        }
        None => {
            let prompt = crate::llm::build_guided_question_check_prompt(
                &step_context(guided, index + 1),
                &questions.join("\n"),
                &crate::settings::load(),
            );
            CheckAnswer::Headless(work::check_questions(root.to_string(), prompt))
        }
    };
    if let Some(commentary) = guided.commentary.get_mut(key) {
        commentary.text = without;
        commentary.done = false;
        commentary.checking = true;
    }
    guided.checks.push(Check {
        key: key.to_string(),
        questions,
        answer,
    });
}

/// One file per step: the key's own hash covers only the changed lines, which
/// two files can share.
fn answer_file_name(key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!("guided-questions-{:016x}.md", hasher.finish())
}

/// Put what each finished check settled back into its step's commentary. A
/// check that failed puts the questions back as they were asked.
fn drain_checks(guided: &mut Guided, notices: &mut Vec<(String, bool)>) {
    let mut finished = Vec::new();
    for (index, check) in guided.checks.iter().enumerate() {
        let polled = match &check.answer {
            CheckAnswer::Headless(rx) => poll_once(rx),
            CheckAnswer::Chat(file) => match std::fs::read_to_string(file) {
                Ok(settled) => {
                    let _ = std::fs::remove_file(file);
                    Poll::Message(Ok(settled))
                }
                Err(_) if guided.chat.is_none() => Poll::Message(Err(
                    "the review chat ended before it settled the questions".to_string(),
                )),
                Err(_) => Poll::Pending,
            },
        };
        let settled = match polled {
            Poll::Pending => continue,
            Poll::Message(Ok(settled)) => settled,
            Poll::Message(Err(message)) => {
                notices.push((format!("checking the questions: {message}"), true));
                check.questions.join("\n")
            }
            Poll::Disconnected => {
                notices.push((stopped_unexpectedly("checking the questions"), true));
                check.questions.join("\n")
            }
        };
        if let Some(commentary) = guided.commentary.get_mut(&check.key) {
            commentary.text = findings::with_settled(&commentary.text, &settled);
            commentary.done = true;
            commentary.checking = false;
        }
        finished.push(index);
    }
    for index in finished.into_iter().rev() {
        guided.checks.remove(index);
    }
}

/// How many requests may run at once. The local server answers one request
/// per session at a time; Claude takes several.
fn concurrency() -> usize {
    match crate::llm::current_provider() {
        crate::llm::LlmProvider::Claude => CONCURRENT_CLAUDE,
        crate::llm::LlmProvider::Mtplx => CONCURRENT_LOCAL,
    }
}

/// Most wanted first: the plan, which orders the walk, then the step on
/// screen. A file is read only once the walk reaches it, the reviewer asks for
/// its concern to be read, or the chat is owed its redo.
fn steps_to_read(guided: &Guided) -> Vec<usize> {
    let owed = |i: usize| {
        let path = &guided.steps[i - 1].path;
        guided.chat_owed.iter().any(|(owed, _)| owed == path)
    };
    let wanted = |i: usize| guided.read_wanted.contains(&guided.steps[i - 1].path);
    let mut seen = HashSet::new();
    std::iter::once(OVERVIEW)
        .chain(std::iter::once(guided.current))
        .chain(guided.walk_order().into_iter().filter(|&i| wanted(i)))
        .chain((1..=guided.total()).filter(|&i| owed(i)))
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
    // A file waits for the plan, which tells it what concern it is part of.
    let planning = guided
        .commentary
        .get(OVERVIEW_KEY)
        .is_some_and(|plan| !plan.done);
    for index in wanted {
        if slots == 0 {
            break;
        }
        if planning && index != OVERVIEW && index != guided.final_step() {
            continue;
        }
        let key = if index == OVERVIEW {
            OVERVIEW_KEY.to_string()
        } else if index == guided.final_step() {
            guided.final_key()
        } else {
            guided.steps[index - 1].key()
        };
        let asked = guided.commentary.contains_key(&key) || guided.failed.contains(&key);
        if asked {
            continue;
        }
        let (rx, prompt_bytes) = if index == OVERVIEW {
            let context = format!(
                "{}\nThe whole diff, file by file:\n{}",
                guided.overview,
                whole_diff(guided)
            );
            let bytes = context.len();
            (work::ask_overview(context), bytes)
        } else if index == guided.final_step() {
            let context = final_context(guided);
            let bytes = context.len();
            (work::ask_final(context), bytes)
        } else {
            // The walk's first brief, so the cached prompt stays the same.
            let shared = guided
                .shared_brief
                .get_or_insert_with(|| guided.brief.clone())
                .clone();
            let context = file_context(guided, index);
            let bytes = shared.len() + context.len();
            (work::ask_step(shared, context), bytes)
        };
        guided.commentary.insert(
            key.clone(),
            Commentary {
                started: Some(std::time::Instant::now()),
                prompt_bytes,
                ..Commentary::default()
            },
        );
        guided.asks.push(Ask {
            target: Target::Commentary(key),
            rx,
        });
        slots -= 1;
    }
}

/// Everything the model is shown about step `index`: the change's overview,
/// the file's diff with every line numbered, and the source around its hunks
/// when the files on disk are the ones being reviewed.
fn step_context(guided: &Guided, index: usize) -> String {
    format!(
        "Change overview:\n{}{}",
        guided.overview,
        file_context(guided, index)
    )
}

fn file_context(guided: &Guided, index: usize) -> String {
    let step = &guided.steps[index - 1];
    let mut out = String::new();
    out.push_str(&format!(
        "\nFile on screen: {} ({} hunk(s)){}\n",
        step.path,
        step.hunks(),
        step.file_note
            .as_deref()
            .map(|note| format!(", {note}"))
            .unwrap_or_default()
    ));
    out.push_str(&numbered_diff(step));
    out.push_str(&concern_context(guided, step));
    if guided.source.editable()
        && let Some(source) = source_around(step)
    {
        out.push_str("\nThe file around its hunks, as it is now:\n");
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

/// The concern the plan puts `step` in, and the diffs of its other files, so
/// the model reads a call site with the definition it calls.
fn concern_context(guided: &Guided, step: &GuidedFile) -> String {
    let Some(plan) = guided.plan() else {
        return String::new();
    };
    let Some(concern) = plan.concerns.iter().find(|concern| {
        concern.title != plan::EVERYTHING_ELSE && concern.paths.contains(&step.path)
    }) else {
        return String::new();
    };
    let why = concern
        .why
        .as_deref()
        .map(|why| format!(": {why}"))
        .unwrap_or_default();
    let mut out = format!(
        "\nThis file is part of the concern \"{}\"{why}\n",
        concern.title
    );
    let others: Vec<&GuidedFile> = guided
        .steps
        .iter()
        .filter(|other| other.path != step.path && concern.paths.contains(&other.path))
        .collect();
    if others.is_empty() {
        return out;
    }
    out.push_str(
        "The concern's other files, for context only: review and cite only the file on screen, \
         but flag where it and they disagree.\n",
    );
    let mut left = CONCERN_CONTEXT_BYTES;
    for other in others {
        let diff = numbered_diff(other);
        if diff.len() > left {
            out.push_str(&format!("\n=== {}: left out, too long\n", other.path));
            continue;
        }
        left -= diff.len();
        out.push_str(&format!("\n=== {}\n{diff}", other.path));
    }
    out
}

/// Every file's numbered diff, as much as fits.
fn whole_diff(guided: &Guided) -> String {
    let mut out = String::new();
    let mut left = WHOLE_DIFF_BYTES;
    for step in &guided.steps {
        let note = step
            .file_note
            .as_deref()
            .map(|note| format!(" ({note})"))
            .unwrap_or_default();
        let diff = numbered_diff(step);
        if diff.len() > left {
            out.push_str(&format!(
                "\n=== {}{note}: left out, the diff is too long to show whole\n",
                step.path
            ));
            continue;
        }
        left -= diff.len();
        out.push_str(&format!("\n=== {}{note}\n{diff}", step.path));
    }
    out
}

/// The file's diff with each line numbered as a finding cites it.
fn numbered_diff(step: &GuidedFile) -> String {
    let mut out = String::new();
    for line in &step.lines {
        let number = line
            .new
            .or(line.old)
            .map(|n| format!("L{n}"))
            .unwrap_or_default();
        out.push_str(&format!("{number:>6} {}\n", line.text));
    }
    out
}

/// The whole changeset for the final review: every file's diff, as much as
/// fits, and what the file-by-file reads have found and not yet had ticked off.
fn final_context(guided: &Guided) -> String {
    let mut out = format!(
        "Change overview:\n{}\nThe whole diff, file by file:\n{}",
        guided.overview,
        whole_diff(guided)
    );
    out.push_str("\nWhat the file-by-file reads raised that is still open:\n");
    for step in &guided.steps {
        let Some(commentary) = guided.commentary.get(&step.key()).filter(|c| c.done) else {
            out.push_str(&format!("- {}: not read yet\n", step.path));
            continue;
        };
        let open = findings::read(&commentary.text)
            .findings
            .into_iter()
            .filter(|finding| !guided.is_ticked(&ticket_in(&step.path, finding)));
        for finding in open {
            out.push_str(&format!(
                "- {}:L{} {}: {}\n",
                step.path,
                finding.line,
                finding.severity.word(),
                finding.text
            ));
        }
    }
    for note in &guided.progress.notes {
        out.push_str(&format!(
            "- {}:L{} reviewer's note: {}\n",
            note.path, note.line, note.body
        ));
    }
    out
}

/// The working file's lines around each hunk, numbered; windows that meet
/// are joined, and the gaps between them marked.
fn source_around(step: &GuidedFile) -> Option<String> {
    let mut windows: Vec<(usize, usize)> = Vec::new();
    let mut hunk: Option<(usize, usize)> = None;
    for line in &step.lines {
        if line.kind == crate::git::guided::LineKind::Header {
            windows.extend(hunk.take());
            continue;
        }
        if let Some(n) = line.new {
            hunk = Some(hunk.map_or((n, n), |(first, _)| (first, n)));
        }
    }
    windows.extend(hunk);
    if windows.is_empty() {
        return None;
    }
    let root = crate::git::repo_dir()?;
    let text = std::fs::read_to_string(root.join(&step.path)).ok()?;
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (first, last) in windows {
        let from = first.saturating_sub(SOURCE_CONTEXT_LINES).max(1);
        let to = last + SOURCE_CONTEXT_LINES;
        match merged.last_mut() {
            Some((_, end)) if from <= *end + 1 => *end = (*end).max(to),
            _ => merged.push((from, to)),
        }
    }
    let mut out = String::new();
    for (at, (from, to)) in merged.into_iter().enumerate() {
        if at > 0 {
            out.push_str("   \u{22ef}\n");
        }
        for (index, line) in text.lines().enumerate().skip(from - 1).take(to + 1 - from) {
            out.push_str(&format!("{:>6} {line}\n", format!("L{}", index + 1)));
        }
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
    guided.current = index.min(guided.final_step());
    guided.cursor = guided.step().map(GuidedFile::first_changed).unwrap_or(0);
    guided.side_scroll = 0;
    guided.final_scroll = 0;
    guided.list_scroll = None;
    if guided.on_final() {
        let noun = if guided.source.is_pull_request() {
            "s submits the review"
        } else {
            "s shows every note"
        };
        state.set_status(
            format!("final review of the whole change \u{2014} {noun}"),
            false,
        );
    }
}

/// The next file in the walk's order; past the last is the final review, and
/// past that every note.
pub(super) fn next_step(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.on_final() {
        open_summary(state);
        return;
    }
    let next = guided.step_after(guided.current);
    go_to(state, next, true);
}

/// The next walk mode. Each orders the same files from the same plan, so
/// switching keeps every note, tick and reviewed mark.
pub(super) fn switch_walk(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    guided.progress.walk = guided.progress.walk.next();
    let walk = guided.progress.walk;
    let waiting = if guided.plan().is_none() {
        " (in path order until the plan is read)"
    } else {
        ""
    };
    match guided.save() {
        Ok(()) => state.set_status(
            format!(
                "walk by {}: {}{waiting}",
                walk.label().to_lowercase(),
                walk.describe()
            ),
            false,
        ),
        Err(message) => state.set_status(message, true),
    }
}

/// Have every file of concern `index` read now, ahead of the walk.
pub(super) fn read_concern(state: &mut AppState, index: usize) {
    if !state.ai_assist {
        state.set_status(crate::panel::commit::AI_OFF_NOTICE, false);
        return;
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(concern) = guided
        .plan()
        .and_then(|plan| plan.concerns.into_iter().nth(index))
    else {
        return;
    };
    let (read, total) = guided.read_of(&concern.paths);
    for path in &concern.paths {
        if !guided.read_wanted.contains(path) {
            guided.read_wanted.push(path.clone());
        }
    }
    let message = if read == total {
        format!("every file of \"{}\" is read", concern.title)
    } else {
        format!(
            "reading the {} file(s) of \"{}\" still unread\u{2026}",
            total - read,
            concern.title
        )
    };
    state.set_status(message, false);
}

/// Have the concern of the file on screen read whole.
pub(super) fn read_concern_on_screen(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    let Some(path) = guided.step().map(|step| step.path.clone()) else {
        state.set_status("the concern is a file's: \u{2192} to one first", false);
        return;
    };
    let Some(plan) = guided.plan() else {
        state.set_status("the plan is not read yet", false);
        return;
    };
    match plan
        .concerns
        .iter()
        .position(|concern| concern.paths.contains(&path))
    {
        Some(index) => read_concern(state, index),
        None => state.set_status("this file is in no concern", false),
    }
}

/// The next open finding, worst first: its file on screen, the cursor on its
/// line. Past the last is the final review, and past that every note.
pub(super) fn next_issue(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.on_final() {
        open_summary(state);
        return;
    }
    let all = guided.findings_in_order();
    let from = finding_here(guided, &all).map_or(0, |at| at + 1);
    let next = all[from..]
        .iter()
        .find(|(_, ticked)| !ticked)
        .map(|(open, _)| open.clone());
    match next {
        Some(finding) => go_to_finding(state, &finding),
        None => {
            go_final(state);
            let read = state.guided.as_deref().map_or(0, |guided| {
                guided
                    .steps
                    .iter()
                    .filter(|step| guided.is_read(step))
                    .count()
            });
            let total = state.guided.as_deref().map_or(0, Guided::total);
            state.set_status(
                format!("no more open findings in the {read} of {total} file(s) read"),
                false,
            );
        }
    }
}

pub(super) fn previous_issue(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    let all = guided.findings_in_order();
    let before = match finding_here(guided, &all) {
        Some(at) => at,
        None if guided.on_final() => all.len(),
        None => 0,
    };
    let previous = all[..before]
        .iter()
        .rev()
        .find(|(_, ticked)| !ticked)
        .map(|(open, _)| open.clone());
    match previous {
        Some(finding) => go_to_finding(state, &finding),
        None => go_to(state, OVERVIEW, false),
    }
}

/// The finding the cursor is on, ticked or not.
fn finding_here(guided: &Guided, all: &[(OpenFinding, bool)]) -> Option<usize> {
    all.iter()
        .position(|(open, _)| open.step == guided.current && open.at == Some(guided.cursor))
}

pub(super) fn go_to_finding(state: &mut AppState, finding: &OpenFinding) {
    go_to(state, finding.step, false);
    if let Some(guided) = state.guided.as_mut()
        && let Some(at) = finding.at
    {
        guided.cursor = at;
    }
}

/// Mark every file the plan says is safe to skim as reviewed.
pub(super) fn mark_skims_reviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(plan) = guided.plan() else {
        state.set_status("the plan is not read yet", false);
        return;
    };
    let keys: Vec<String> = guided
        .steps
        .iter()
        .filter(|step| plan.skim.contains_key(&step.path))
        .map(GuidedFile::key)
        .filter(|key| !guided.progress.reviewed.contains(key))
        .collect();
    if keys.is_empty() {
        state.set_status("no files left to skim", false);
        return;
    }
    let count = keys.len();
    guided.progress.reviewed.extend(keys);
    match guided.save() {
        Ok(()) => state.set_status(format!("{count} file(s) to skim marked reviewed"), false),
        Err(message) => state.set_status(message, true),
    }
}

pub(super) fn go_final(state: &mut AppState) {
    if let Some(last) = state.guided.as_deref().map(Guided::final_step) {
        go_to(state, last, false);
    }
}

/// The first press only asks: it forgets every note, which the walk cannot give back.
pub(super) fn reset_review(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !std::mem::take(&mut guided.reset_armed) {
        guided.reset_armed = true;
        let notes = guided.progress.notes.len();
        state.set_status(
            format!(
                "reset again to start over: forgets {notes} note(s), every reviewed mark and tick, and the model's reads"
            ),
            false,
        );
        return;
    }
    guided.progress = GuidedProgress {
        walk: guided.progress.walk,
        ..GuidedProgress::default()
    };
    guided.commentary.clear();
    guided.exchanges.clear();
    guided.failed.clear();
    guided.earlier.clear();
    guided.asks.clear();
    guided.checks.clear();
    guided.check_queue.clear();
    guided.read_wanted.clear();
    guided.chat_owed.clear();
    guided.shared_brief = None;
    guided.delete_armed = None;
    guided.mode = Mode::Browse;
    guided.current = OVERVIEW;
    guided.cursor = 0;
    guided.side_scroll = 0;
    guided.final_scroll = 0;
    match guided.save() {
        Ok(()) => state.set_status(
            "review reset \u{2014} starting over from the overview",
            false,
        ),
        Err(message) => state.set_status(message, true),
    }
}

/// Scrolls the whole changeset on the final step, as the cursor scrolls a file.
pub(super) fn scroll_final(state: &mut AppState, by: i32) {
    if let Some(guided) = state.guided.as_mut() {
        guided.final_scroll = guided.final_scroll.min(guided.final_max.get());
        guided.final_scroll = if by >= 0 {
            guided.final_scroll.saturating_add(by as u16)
        } else {
            guided.final_scroll.saturating_sub(by.unsigned_abs() as u16)
        };
    }
}

pub(super) fn previous_step(state: &mut AppState) {
    let Some(previous) = state
        .guided
        .as_deref()
        .map(|guided| guided.step_before(guided.current))
    else {
        return;
    };
    go_to(state, previous, false);
}

pub(super) fn next_unreviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    let order = guided.walk_order();
    let here = order
        .iter()
        .position(|&step| step == guided.current)
        .map_or(0, |at| at + 1);
    let after = order[here..]
        .iter()
        .chain(&order[..here])
        .copied()
        .find(|&step| !guided.is_reviewed(&guided.steps[step - 1]));
    match after {
        Some(index) => go_to(state, index, false),
        None => state.set_status("every file is reviewed", false),
    }
}

pub(super) fn toggle_reviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(key) = guided.step().map(GuidedFile::key) else {
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
    if guided.on_final() {
        scroll_final(state, if down { 1 } else { -1 });
        return;
    }
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
    if running.is_some() {
        guided.mode = Mode::Chat;
        state.set_status("chat \u{2014} Ctrl-] back to the review", false);
        return;
    }
    if guided.step().is_none() {
        state.set_status("the chat is about a file: \u{2192} to one first", false);
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
                state.set_status("notes go on a file's lines", false);
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
    } else if guided.on_final() {
        final_context(guided)
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
    let Some(step) = guided.step() else {
        let key = if guided.on_final() {
            guided.final_key()
        } else {
            OVERVIEW_KEY.to_string()
        };
        forget(guided, &[key]);
        reload(state);
        return;
    };
    let (path, key) = (step.path.clone(), step.key());
    if let Some(commentary) = guided.commentary.get(&key).filter(|c| c.done) {
        guided
            .earlier
            .insert(path.clone(), commentary.text.trim().to_string());
    }
    forget(guided, &[key]);
    if guided.chat.is_some() {
        guided.owe_chat(path.clone(), ChatNews::Redone);
    }
    state.set_status(format!("reviewing {path} again\u{2026}"), false);
    reload(state);
}

fn forget(guided: &mut Guided, keys: &[String]) {
    guided.asks.retain(|ask| match &ask.target {
        Target::Commentary(key) => !keys.contains(key),
        Target::Answer(..) => true,
    });
    guided.checks.retain(|check| !keys.contains(&check.key));
    guided.check_queue.retain(|key| !keys.contains(key));
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
        state.set_status("pick a file to edit", false);
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
        state.guided = Some(Box::new(Guided::new(Source::Unpushed {
            branch: "feature".into(),
            base_ref: String::new(),
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
        let mut guided = Guided::new(Source::Unpushed {
            branch: "feature".into(),
            base_ref: String::new(),
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
        guided.steps = crate::git::guided::parse_files(&diff);
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
    fn each_file_is_one_row_that_opens_it() {
        let mut state = walk_on_a_long_hunk();
        let two_hunk_file = |name: &str| {
            format!(
                "diff --git a/{name} b/{name}\n--- a/{name}\n+++ b/{name}\n@@ -1 +1 @@ fn {name}\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n"
            )
        };
        let guided = state.guided.as_mut().unwrap();
        guided.steps = crate::git::guided::parse_files(&format!(
            "{}{}",
            two_hunk_file("first.rs"),
            two_hunk_file("second.rs")
        ));
        guided.current = OVERVIEW;
        let (list, _) = panes(&state);

        // The overview, first.rs, second.rs: the third row is the second file.
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            list.x + 2,
            list.y + 2,
        );

        let guided = state.guided.as_deref().unwrap();
        assert_eq!(
            guided.step().map(|step| step.path.as_str()),
            Some("second.rs")
        );
    }

    #[test]
    fn the_step_list_shows_the_file_tree_and_a_directory_row_opens_nothing() {
        let mut state = walk_on_a_long_hunk();
        let file = |path: &str| {
            format!(
                "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n"
            )
        };
        let guided = state.guided.as_mut().unwrap();
        guided.steps =
            crate::git::guided::parse_files(&format!("{}{}", file("src/b.rs"), file("src/a/x.rs")));
        guided.current = OVERVIEW;
        let (list, _) = panes(&state);
        let rows = screen(&state);
        let row = |y: u16| rows[y as usize][..].to_string();
        assert!(row(list.y + 1).contains("src/"), "{}", row(list.y + 1));
        assert!(row(list.y + 2).contains("a/"), "{}", row(list.y + 2));
        assert!(row(list.y + 3).contains("x.rs"), "{}", row(list.y + 3));
        assert!(row(list.y + 4).contains("b.rs"), "{}", row(list.y + 4));
        let click = |state: &mut AppState, y: u16| {
            mouse(
                state,
                MouseEventKind::Down(MouseButton::Left),
                list.x + 4,
                y,
            )
        };

        click(&mut state, list.y + 1);
        assert_eq!(state.guided.as_ref().unwrap().current, OVERVIEW);

        click(&mut state, list.y + 3);
        let guided = state.guided.as_deref().unwrap();
        assert_eq!(guided.step().map(|s| s.path.as_str()), Some("src/a/x.rs"));
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
    fn b_switches_between_the_whole_branch_and_what_is_not_pushed() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE);
        let mut state = walk_on_a_long_hunk();

        handle_key(&mut state, b).unwrap();
        assert_eq!(
            state.guided.as_ref().unwrap().source,
            Source::Unpushed {
                branch: "guided-test-branch".into(),
                base_ref: String::new(),
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
    /// over what is not pushed stays put and says why.
    #[test]
    fn the_base_branch_has_only_what_is_not_pushed_to_walk() {
        let mut state = AppState::new();
        let branch = crate::preferences::base_branch();
        let unpushed = Source::Unpushed {
            branch,
            base_ref: String::new(),
        };
        state.guided = Some(Box::new(Guided::new(unpushed.clone())));
        state.modal = Modal::GuidedReview;

        switch_scope(&mut state);

        assert_eq!(state.guided.as_ref().unwrap().source, unpushed);
        assert!(state.status.is_some_and(|status| status.is_error));
    }

    #[test]
    fn the_overview_lists_each_file_once_with_its_totals() {
        let steps = crate::git::guided::parse_files(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,2 @@\n x\n+y\n@@ -9,1 +10,1 @@\n-p\n+q\n",
        );
        let Overview { text, brief } = overview_of("Branch b", "", &["abc fix it".into()], &steps);

        assert!(text.contains("- abc fix it"));
        assert_eq!(text.matches("a.rs").count(), 1, "{text}");
        assert!(text.contains("+2 -1"), "{text}");
        assert!(brief.contains("- a.rs\n"), "{brief}");
        assert!(!brief.contains("+2 -1"), "{brief}");
    }

    /// a.rs of two hunks and b.rs of one. Both start reviewed, so walking
    /// past them saves nothing to disk.
    fn walk_with_findings() -> AppState {
        let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
                    @@ -1,3 +1,3 @@\n keep\n-old one\n+new one\n tail\n\
                    @@ -20,3 +20,3 @@\n keep\n-old two\n+new two\n keep\n\
                    diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        let mut guided = Guided::new(Source::Branch {
            branch: "guided-test-findings".into(),
            base_ref: "main".into(),
        });
        guided.steps = crate::git::guided::parse_files(diff);
        guided.progress.reviewed = guided.steps.iter().map(GuidedFile::key).collect();
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
            commentary("Summary: Renames x.\nL1 nit: y says less than x.\nVerdict: nit"),
        );
        guided.current = 1;
        guided.cursor = guided.steps[0].first_changed();
        let mut state = AppState::new();
        state.guided = Some(Box::new(guided));
        state.modal = Modal::GuidedReview;
        state
    }

    /// Step one of `walk_with_findings` with `questions` out with Claude, and
    /// the end of the channel its answer comes in on.
    fn checking(
        guided: &mut Guided,
        questions: &[&str],
    ) -> std::sync::mpsc::Sender<Result<String, String>> {
        let key = guided.steps[0].key();
        guided.commentary.insert(
            key.clone(),
            Commentary {
                text: "Summary: Renames a line.\nVerdict: ok".into(),
                checking: true,
                ..Default::default()
            },
        );
        let (tx, rx) = std::sync::mpsc::channel();
        guided.checks.push(Check {
            key,
            questions: questions.iter().map(|q| q.to_string()).collect(),
            answer: CheckAnswer::Headless(rx),
        });
        tx
    }

    fn open_questions(guided: &Guided) -> Vec<String> {
        let commentary = &guided.commentary[&guided.steps[0].key()];
        findings::read(&commentary.text)
            .findings
            .into_iter()
            .filter(|finding| finding.severity == findings::Severity::Question)
            .map(|finding| finding.text)
            .collect()
    }

    #[test]
    fn only_the_questions_the_code_cannot_settle_are_left_for_the_reviewer() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        let tx = checking(
            guided,
            &[
                "L2 question: is old one read anywhere else?",
                "L3 question: should the rename change the output?",
            ],
        );
        assert!(!guided.has_read("a.rs"), "its questions are still out");

        tx.send(Ok(
            "L3 question: should the rename change the output?".into()
        ))
        .unwrap();
        drain_checks(guided, &mut Vec::new());

        assert!(guided.has_read("a.rs"));
        assert_eq!(
            open_questions(guided),
            vec!["should the rename change the output?"]
        );
    }

    #[test]
    fn a_check_that_fails_leaves_the_questions_as_they_were_asked() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        let tx = checking(guided, &["L2 question: is old one read anywhere else?"]);

        tx.send(Err("claude was not found on PATH".into())).unwrap();
        let mut notices = Vec::new();
        drain_checks(guided, &mut notices);

        assert!(guided.has_read("a.rs"));
        assert_eq!(
            open_questions(guided),
            vec!["is old one read anywhere else?"]
        );
        assert!(notices.iter().any(|(_, is_error)| *is_error));
    }

    #[test]
    fn an_open_chat_settles_the_questions_through_the_file_it_was_told_to_write() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        drop(checking(guided, &[]));
        let file =
            std::env::temp_dir().join(format!("lg-guided-questions-{}.md", std::process::id()));
        let _ = std::fs::remove_file(&file);
        guided.checks[0].questions = vec![
            "L2 question: is old one read anywhere else?".into(),
            "L3 question: should the rename change the output?".into(),
        ];
        guided.checks[0].answer = CheckAnswer::Chat(file.clone());
        guided.chat = Some(crate::session::SessionId::unstarted(1));

        drain_checks(guided, &mut Vec::new());
        assert!(!guided.has_read("a.rs"), "the chat has not answered yet");

        std::fs::write(&file, "L2 nit: old one is still read by the export.\n").unwrap();
        drain_checks(guided, &mut Vec::new());

        assert!(guided.has_read("a.rs"));
        assert!(open_questions(guided).is_empty());
        let commentary = &guided.commentary[&guided.steps[0].key()];
        assert!(
            commentary
                .text
                .contains("old one is still read by the export.")
        );
    }

    #[test]
    fn a_chat_that_ends_before_answering_leaves_the_questions_as_they_were_asked() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        drop(checking(
            guided,
            &["L2 question: is old one read anywhere else?"],
        ));
        guided.checks[0].answer =
            CheckAnswer::Chat(std::env::temp_dir().join("lg-guided-questions-never-written.md"));

        drain_checks(guided, &mut Vec::new());

        assert!(guided.has_read("a.rs"));
        assert_eq!(
            open_questions(guided),
            vec!["is old one read anywhere else?"]
        );
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
    fn the_arrows_walk_the_flagged_lines_across_files() {
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
        assert_eq!(cursor_line(&state), (2, "+y".into()));
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

    /// A review chat on a.rs running `script` in place of claude, once it has
    /// printed `ready`.
    fn chatting(state: &mut AppState, script: &str, ready: &str) {
        let spawn = crate::term::Spawn {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            env_remove: Vec::new(),
        };
        let spec = crate::session::SessionSpec {
            label: "review chat".into(),
            cwd: std::env::temp_dir(),
            sandboxed: false,
            kind: crate::session::SessionKind::Claude,
            prompt: None,
        };
        let id = state.sessions.start_with(spec, &spawn, (24, 80)).unwrap();
        let started = std::time::Instant::now();
        while !state
            .sessions
            .get(id)
            .is_some_and(|session| session.screen().contents().contains(ready))
        {
            assert!(
                started.elapsed().as_secs() < 10,
                "the chat never printed {ready}"
            );
            state.sessions.pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let guided = state.guided.as_mut().unwrap();
        guided.chat = Some(id);
        guided.chat_file = Some("a.rs".into());
    }

    #[test]
    fn a_drag_started_in_the_chat_copies_only_the_chat() {
        let mut state = walk_with_findings();
        chatting(&mut state, "echo chat says hello; sleep 30", "hello");
        let (_, side) = panes(&state);
        let chat =
            draw::chat_screen_area(state.guided.as_deref().unwrap(), side, state.diff_view_mode)
                .unwrap();

        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            chat.x,
            chat.y,
        );
        mouse(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            0,
            AREA.height - 1,
        );
        mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            0,
            AREA.height - 1,
        );

        let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(&state, AREA, frame)).unwrap();
        state.sessions.close_all();
        let copied = state.selection.unwrap().text(terminal.backend().buffer());
        assert!(copied.contains("chat says hello"), "{copied:?}");
        assert!(
            !copied.contains("Renames"),
            "the read beside it: {copied:?}"
        );
        assert!(!copied.contains("new one"), "the diff above it: {copied:?}");
    }

    #[test]
    fn a_running_chat_stays_on_screen_through_the_final_review() {
        let mut state = walk_with_findings();
        chatting(&mut state, "echo ready; sleep 30", "ready");

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('w'));

        assert!(state.guided.as_ref().unwrap().on_final());
        let rows = screen(&state);
        state.sessions.close_all();
        assert!(
            rows.iter().any(|row| row.contains("Claude \u{b7} a.rs")),
            "{rows:#?}"
        );
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

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('C'));

        assert_eq!(state.pending_action, Some(PendingAction::GuidedChat));
        let brief = state.guided.as_ref().unwrap().chat_brief("a.rs");
        assert!(
            brief.contains("L2 issue: -1 stands in for missing"),
            "{brief}"
        );
        assert!(!brief.contains("tail could say"), "{brief}");
        assert!(!brief.contains("y says less"), "{brief}");

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('Y'));
        let Some(PendingAction::CopyToClipboard { text, .. }) = &state.pending_action else {
            panic!("Y copies");
        };
        assert_eq!(*text, brief);
    }

    #[test]
    fn the_plan_is_read_first_and_a_file_only_once_the_walk_reaches_it() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();

        guided.current = 1;
        assert_eq!(steps_to_read(guided), vec![OVERVIEW, 1]);
        guided.current = OVERVIEW;
        assert_eq!(steps_to_read(guided), vec![OVERVIEW]);
        guided.current = guided.final_step();
        assert_eq!(steps_to_read(guided), vec![OVERVIEW, guided.final_step()]);

        guided.owe_chat("a.rs".into(), ChatNews::FileRead);
        guided.owe_chat("a.rs".into(), ChatNews::Redone);
        guided.owe_chat("a.rs".into(), ChatNews::FileRead);
        assert_eq!(
            guided.chat_owed,
            vec![("a.rs".to_string(), ChatNews::Redone)]
        );
        guided.current = 2;
        assert_eq!(steps_to_read(guided), vec![OVERVIEW, 2, 1]);
    }

    #[test]
    fn ticking_findings_off_settles_the_file() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        let first = guided.steps[0].clone();
        assert_eq!(guided.verdict(&first), Some(Verdict::Issue));
        let open = |guided: &Guided| {
            (
                guided.open_in_file("a.rs", findings::Severity::Issue),
                guided.open_in_file("a.rs", findings::Severity::Nit),
            )
        };
        assert_eq!(open(guided), (1, 1));

        guided.progress.ticked.push(TickedFinding {
            path: "a.rs".into(),
            line: 2,
            text: "-1 stands in for missing and is used as an index.".into(),
        });
        assert_eq!(guided.verdict(&first), Some(Verdict::Nit));
        assert_eq!(open(guided), (0, 1));

        guided.progress.ticked.push(TickedFinding {
            path: "a.rs".into(),
            line: 3,
            text: "tail could say what it ends.".into(),
        });
        assert_eq!(guided.verdict(&first), Some(Verdict::Ok));
        assert_eq!(open(guided), (0, 0));

        // An open question is not a defect: the hunk stays ok.
        let key = first.key();
        guided.commentary.insert(
            key,
            commentary("L2 question: can the index be -1 here?\nVerdict: ok"),
        );
        assert_eq!(guided.verdict(&first), Some(Verdict::Ok));
        assert_eq!(guided.open_in_file("a.rs", findings::Severity::Question), 1);
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
        assert!(file_context(guided, 1).contains("-1 stands in for missing"));

        let guided = state.guided.as_mut().unwrap();
        assert!(!guided.has_read("a.rs"));
        guided.loading = None;
        let key = guided.steps[0].key();
        guided.commentary.insert(
            key,
            commentary("Summary: Same.\nFixed: the index sentinel.\nVerdict: ok"),
        );
        assert!(guided.has_read("a.rs"));
        assert!(
            guided.has_read("gone.rs"),
            "a file the change no longer has"
        );
        assert!(
            guided
                .chat_brief("a.rs")
                .contains("fixed: the index sentinel.")
        );
    }

    #[test]
    fn the_file_follows_the_diff_layout_with_findings_under_their_line() {
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
    fn the_walk_says_whether_it_covers_the_branch_or_only_what_is_not_pushed() {
        let shows = |source: Source| {
            let mut state = AppState::new();
            state.guided = Some(Box::new(Guided::new(source)));
            state.modal = Modal::GuidedReview;
            let rows = screen(&state);
            (
                rows.iter().any(|row| row.contains("BRANCH")),
                rows.iter().any(|row| row.contains("NOT PUSHED")),
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
            shows(Source::Unpushed {
                branch: "guided-test-scope".into(),
                base_ref: "origin/guided-test-scope".into(),
            }),
            (false, true)
        );
    }

    #[test]
    fn past_the_last_file_is_the_final_review_and_past_that_every_note() {
        use ratatui::crossterm::event::KeyCode::Right;
        let mut state = walk_with_findings();
        state.guided.as_mut().unwrap().current = 2;

        press(&mut state, Right);
        let guided = state.guided.as_ref().unwrap();
        assert!(guided.on_final());
        assert!(guided.is_reviewed(&guided.steps[1]));

        press(&mut state, Right);
        assert!(matches!(
            state.guided.as_ref().unwrap().mode,
            Mode::Summary { .. }
        ));
    }

    #[test]
    fn the_final_review_is_shown_the_whole_change_and_what_is_still_open() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        guided.progress.ticked.push(TickedFinding {
            path: "a.rs".into(),
            line: 3,
            text: "tail could say what it ends.".into(),
        });

        let context = final_context(guided);

        for line in ["+new one", "+new two", "+y"] {
            assert!(context.contains(line), "{line} missing from\n{context}");
        }
        assert!(context.contains("-1 stands in for missing"), "{context}");
        assert!(context.contains("y says less than x"), "{context}");
        assert!(!context.contains("tail could say"), "{context}");
    }

    /// The final step shows every file's diff with the findings across files
    /// under their lines, and a click on one ticks it off.
    #[test]
    fn the_final_step_pins_findings_across_files_into_the_whole_change() {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        guided.current = guided.final_step();
        let key = guided.final_key();
        guided.commentary.insert(
            key,
            commentary(
                "Summary: Two renames that agree.\n\
                 b.rs:L1 issue: callers of x are not updated.\n\
                 Verdict: issue",
            ),
        );
        assert_eq!(
            state.guided.as_ref().unwrap().final_verdict(),
            Some(Verdict::Issue)
        );

        let rows = screen(&state);
        let row_of = |text: &str| rows.iter().position(|row| row.contains(text));
        let (_, side) = panes(&state);
        let guided = state.guided.as_deref().unwrap();
        let diff = draw::diff_area(guided, side, state.diff_view_mode).unwrap();
        assert!(row_of("new one").is_some() && row_of("new two").is_some());
        let pinned = (diff.y..diff.y + diff.height)
            .find(|&y| rows[y as usize].contains("callers of x are not updated"))
            .expect("the finding is pinned in the whole change");
        assert!(pinned as usize > row_of("b.rs").unwrap());

        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            100,
            pinned,
        );
        mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            100,
            pinned,
        );

        assert_eq!(
            state.guided.as_ref().unwrap().final_verdict(),
            Some(Verdict::Ok)
        );
    }

    fn reviewed_with_a_note() -> AppState {
        let mut state = walk_with_findings();
        let guided = state.guided.as_mut().unwrap();
        guided.progress.notes.push(GuidedNote {
            path: "a.rs".into(),
            line: 2,
            side: Side::Right,
            body: "why?".into(),
            step: guided.steps[0].key(),
            quote: "+new one".into(),
        });
        guided.current = 2;
        state
    }

    fn was_reset(state: &AppState) -> bool {
        let guided = state.guided.as_ref().unwrap();
        guided.progress == GuidedProgress::default()
            && guided.commentary.is_empty()
            && guided.current == OVERVIEW
    }

    #[test]
    fn x_resets_the_review_only_when_pressed_twice_in_a_row() {
        use ratatui::crossterm::event::KeyCode::Char;
        let mut state = reviewed_with_a_note();

        press(&mut state, Char('X'));
        assert!(!was_reset(&state), "the first press only asks");
        press(&mut state, Char('j'));
        press(&mut state, Char('X'));
        assert!(!was_reset(&state), "another key in between disarms it");

        press(&mut state, Char('X'));
        assert!(was_reset(&state));
    }

    #[test]
    fn the_reset_button_under_the_steps_resets_on_a_second_click() {
        let mut state = reviewed_with_a_note();
        let (list, _) = panes(&state);
        let button = draw::reset_button_area(list);
        let click = |state: &mut AppState| {
            mouse(
                state,
                MouseEventKind::Down(MouseButton::Left),
                button.x + 2,
                button.y,
            )
        };

        click(&mut state);
        assert!(!was_reset(&state));
        assert!(screen(&state)[button.y as usize].contains("click again"));

        click(&mut state);
        assert!(was_reset(&state));
    }

    #[test]
    fn a_click_on_the_scope_badge_switches_scope() {
        let mut state = walk_on_a_long_hunk();
        let badge = draw::scope_badge_area(state.guided.as_deref().unwrap(), AREA).unwrap();

        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            badge.x + 1,
            badge.y,
        );

        assert!(matches!(
            state.guided.as_ref().unwrap().source,
            Source::Unpushed { .. }
        ));
    }

    /// Four files and the plan the model gave for them. Progress starts empty
    /// whatever an earlier run saved, since the tests save as they walk.
    fn walk_with_plan() -> AppState {
        let file = |path: &str, line: &str| {
            format!(
                "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-old\n+{line}\n"
            )
        };
        let mut guided = Guided::new(Source::Branch {
            branch: "guided-test-modes".into(),
            base_ref: "main".into(),
        });
        guided.progress = GuidedProgress::default();
        guided.steps = crate::git::guided::parse_files(&format!(
            "{}{}{}{}",
            file("src/lookup.rs", "lookup calls the cache"),
            file("Cargo.lock", "bumped"),
            file("tests/cache.rs", "evicts the oldest"),
            file("src/cache.rs", "fn evict()"),
        ));
        guided.commentary.insert(
            OVERVIEW_KEY.to_string(),
            commentary(
                "Summary: Adds a cache in front of lookup.\n\
                 Concern: The cache\n\
                 Why: Keeps recent lookups.\n\
                 Files: src/lookup.rs, src/cache.rs\n\
                 Concern: Tests\n\
                 Files: tests/cache.rs\n\
                 Order: tests/cache.rs, src/cache.rs, src/lookup.rs\n\
                 Risk: src/lookup.rs high: a new call path\n\
                 Risk: src/cache.rs medium: eviction under the lock\n\
                 Risk: tests/cache.rs low: adds cases\n\
                 Skim: Cargo.lock: dependency bump",
            ),
        );
        let mut state = AppState::new();
        state.guided = Some(Box::new(guided));
        state.modal = Modal::GuidedReview;
        state
    }

    fn walked(state: &AppState) -> Vec<String> {
        let guided = state.guided.as_deref().unwrap();
        guided
            .walk_order()
            .into_iter()
            .map(|step| guided.steps[step - 1].path.clone())
            .collect()
    }

    #[test]
    fn by_concern_the_walk_goes_concern_by_concern_with_each_listed_over_its_files() {
        use ratatui::crossterm::event::KeyCode::Right;
        let mut state = walk_with_plan();

        assert_eq!(
            walked(&state),
            [
                "src/cache.rs",
                "src/lookup.rs",
                "tests/cache.rs",
                "Cargo.lock"
            ]
        );
        press(&mut state, Right);
        let guided = state.guided.as_deref().unwrap();
        assert_eq!(guided.step().map(|s| s.path.as_str()), Some("src/cache.rs"));

        let (list, _) = panes(&state);
        let rows: Vec<String> = screen(&state)
            .into_iter()
            .skip(list.y as usize)
            .take(list.height as usize)
            .map(|row| {
                row.chars()
                    .skip(list.x as usize)
                    .take(list.width as usize)
                    .collect()
            })
            .collect();
        let row_of = |text: &str| {
            rows.iter()
                .position(|row| row.contains(text))
                .unwrap_or_else(|| panic!("{text} is not listed: {rows:#?}"))
        };
        assert!(row_of("The cache") < row_of("cache.rs"));
        assert!(row_of("cache.rs") < row_of("lookup.rs"));
        assert!(row_of("lookup.rs") < row_of("Tests"));
        assert!(row_of("Tests") < row_of("tests/"));
        assert!(row_of("tests/") < row_of(plan::EVERYTHING_ELSE));
        assert!(row_of(plan::EVERYTHING_ELSE) < row_of("Cargo.lock"));
    }

    #[test]
    fn on_the_map_the_walk_follows_the_plans_reading_order() {
        let mut state = walk_with_plan();
        state.guided.as_mut().unwrap().progress.walk = WalkMode::Map;

        assert_eq!(
            walked(&state),
            [
                "tests/cache.rs",
                "src/cache.rs",
                "src/lookup.rs",
                "Cargo.lock"
            ]
        );
    }

    #[test]
    fn triage_walks_the_riskiest_first_and_s_marks_the_files_to_skim_reviewed() {
        use ratatui::crossterm::event::KeyCode::Char;
        let mut state = walk_with_plan();
        state.guided.as_mut().unwrap().progress.walk = WalkMode::Triage;

        assert_eq!(
            walked(&state),
            [
                "src/lookup.rs",
                "src/cache.rs",
                "tests/cache.rs",
                "Cargo.lock"
            ]
        );

        press(&mut state, Char('S'));
        let guided = state.guided.as_deref().unwrap();
        let reviewed: Vec<&str> = guided
            .steps
            .iter()
            .filter(|step| guided.is_reviewed(step))
            .map(|step| step.path.as_str())
            .collect();
        assert_eq!(reviewed, ["Cargo.lock"]);
    }

    #[test]
    fn without_a_plan_every_mode_walks_the_file_tree() {
        let mut state = walk_with_plan();
        let guided = state.guided.as_mut().unwrap();
        guided.commentary.clear();
        for walk in [WalkMode::Concerns, WalkMode::Map, WalkMode::Triage] {
            state.guided.as_mut().unwrap().progress.walk = walk;
            assert_eq!(
                walked(&state),
                [
                    "Cargo.lock",
                    "src/cache.rs",
                    "src/lookup.rs",
                    "tests/cache.rs"
                ]
            );
        }
    }

    /// One click, no confirm: every mode orders the same files, so switching
    /// loses nothing.
    #[test]
    fn a_click_on_the_walk_badge_switches_mode_and_keeps_what_was_done() {
        let mut state = walk_with_plan();
        let guided = state.guided.as_mut().unwrap();
        let reviewed = guided.steps[0].key();
        guided.progress.reviewed.push(reviewed.clone());
        let badge = draw::walk_badge_area(state.guided.as_deref().unwrap(), AREA).unwrap();
        let click = |state: &mut AppState| {
            mouse(
                state,
                MouseEventKind::Down(MouseButton::Left),
                badge.x + 1,
                badge.y,
            )
        };

        click(&mut state);
        assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Map);
        assert!(screen(&state)[badge.y as usize].contains("MAP"));
        click(&mut state);
        assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Triage);
        press(&mut state, ratatui::crossterm::event::KeyCode::Char('M'));
        assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Issues);
        press(&mut state, ratatui::crossterm::event::KeyCode::Char('M'));
        assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Concerns);

        let guided = state.guided.as_ref().unwrap();
        assert!(guided.progress.reviewed.contains(&reviewed));
        assert_eq!(
            crate::git::guided::load_progress(&guided.source.progress_key()).walk,
            WalkMode::Concerns
        );
    }

    #[test]
    fn a_file_is_read_with_the_rest_of_its_concern() {
        let state = walk_with_plan();
        let guided = state.guided.as_deref().unwrap();
        let lookup = guided
            .steps
            .iter()
            .position(|step| step.path == "src/lookup.rs")
            .unwrap();

        let context = file_context(guided, lookup + 1);

        assert!(context.contains("The cache"), "{context}");
        assert!(context.contains("+fn evict()"), "{context}");
        assert!(!context.contains("+evicts the oldest"), "{context}");
    }

    #[test]
    fn files_wait_for_the_plan_while_it_is_read() {
        let mut state = walk_with_plan();
        let guided = state.guided.as_mut().unwrap();
        guided
            .commentary
            .insert(OVERVIEW_KEY.to_string(), Commentary::default());
        guided.current = 1;

        start_wanted_commentary(guided);

        assert!(!guided.commentary.contains_key(&guided.steps[0].key()));
        assert!(guided.asks.is_empty());
    }

    /// The step list's rows as drawn, its own columns only.
    fn list_text(state: &AppState) -> Vec<String> {
        let (list, _) = panes(state);
        screen(state)
            .into_iter()
            .skip(list.y as usize)
            .take(list.height as usize)
            .map(|row| {
                row.chars()
                    .skip(list.x as usize)
                    .take(list.width as usize)
                    .collect()
            })
            .collect()
    }

    fn row_of(rows: &[String], text: &str) -> usize {
        rows.iter()
            .position(|row| row.contains(text))
            .unwrap_or_else(|| panic!("{text} is not listed: {rows:#?}"))
    }

    #[test]
    fn triage_lists_the_files_in_risk_bands_and_the_map_numbers_them() {
        let mut state = walk_with_plan();
        state.guided.as_mut().unwrap().progress.walk = WalkMode::Triage;
        let rows = list_text(&state);
        assert!(row_of(&rows, "High risk") < row_of(&rows, "lookup.rs"));
        assert!(row_of(&rows, "lookup.rs") < row_of(&rows, "Medium risk"));
        assert!(row_of(&rows, "Medium risk") < row_of(&rows, "cache.rs"));
        assert!(row_of(&rows, "Safe to skim") < row_of(&rows, "Cargo.lock"));

        state.guided.as_mut().unwrap().progress.walk = WalkMode::Map;
        let rows = list_text(&state);
        assert!(!rows.iter().any(|row| row.contains("risk")));
        // Numbered in the plan's order: the tests first, the lockfile last.
        assert!(row_of(&rows, "tests/") < row_of(&rows, "1 cache.rs"));
        assert!(rows.iter().any(|row| row.contains("4 Cargo.lock")));
    }

    /// The button reads the whole concern; a click on the heading's title
    /// does nothing, so reading is always chosen.
    #[test]
    fn a_concerns_button_reads_all_of_its_files_and_only_the_button_does() {
        let mut state = walk_with_plan();
        state.ai_assist = true;
        let (list, _) = panes(&state);
        let rows = list_text(&state);
        let heading = list.y + row_of(&rows, "The cache") as u16;
        assert!(rows[row_of(&rows, "The cache")].contains("read all"));
        let click = |state: &mut AppState, column: u16| {
            mouse(
                state,
                MouseEventKind::Down(MouseButton::Left),
                column,
                heading,
            )
        };

        click(&mut state, list.x + 3);
        assert!(!state.guided.as_ref().unwrap().wants_read("src/cache.rs"));

        click(&mut state, list.x + list.width - 2);
        let guided = state.guided.as_deref().unwrap();
        assert!(guided.wants_read("src/cache.rs") && guided.wants_read("src/lookup.rs"));
        assert!(!guided.wants_read("tests/cache.rs"));
        let number = |path: &str| guided.steps.iter().position(|s| s.path == path).unwrap() + 1;
        let wanted = steps_to_read(guided);
        assert!(wanted.contains(&number("src/cache.rs")));
        assert!(wanted.contains(&number("src/lookup.rs")));
        assert!(!wanted.contains(&number("tests/cache.rs")));
    }

    #[test]
    fn a_reads_the_concern_of_the_file_on_screen() {
        let mut state = walk_with_plan();
        state.ai_assist = true;
        let guided = state.guided.as_mut().unwrap();
        guided.current = guided
            .steps
            .iter()
            .position(|s| s.path == "tests/cache.rs")
            .unwrap()
            + 1;

        press(&mut state, ratatui::crossterm::event::KeyCode::Char('A'));

        let guided = state.guided.as_deref().unwrap();
        assert!(guided.wants_read("tests/cache.rs"));
        assert!(!guided.wants_read("src/cache.rs"));
    }

    /// Reads of three files and the final review, each with findings of
    /// their own, walked by issue.
    fn walk_with_findings_read() -> AppState {
        let mut state = walk_with_plan();
        let guided = state.guided.as_mut().unwrap();
        let key = |guided: &Guided, path: &str| {
            guided.steps.iter().find(|s| s.path == path).unwrap().key()
        };
        let reads = [
            (
                key(guided, "src/cache.rs"),
                "L1 nit: evict could say what it evicts.\nVerdict: nit",
            ),
            (
                key(guided, "src/lookup.rs"),
                "L1 question: is the cache warm here?\nVerdict: ok",
            ),
            (
                key(guided, "tests/cache.rs"),
                "L1 issue: the test never fills the cache.\nVerdict: issue",
            ),
        ];
        for (key, text) in reads {
            guided.commentary.insert(key, commentary(text));
        }
        let final_key = guided.final_key();
        guided.commentary.insert(
            final_key,
            commentary(
                "src/lookup.rs:L1 issue: lookup and evict take the lock in opposite order.\n\
                 Verdict: issue",
            ),
        );
        guided.progress.walk = WalkMode::Issues;
        state
    }

    #[test]
    fn issues_lists_every_open_finding_worst_first_and_the_arrows_walk_them() {
        use ratatui::crossterm::event::KeyCode::{Char, Right};
        let mut state = walk_with_findings_read();

        let rows = list_text(&state);
        // The list is narrow: a finding shows where it is and how it starts.
        assert!(row_of(&rows, "Issues (2)") < row_of(&rows, "cache.rs:1 the test"));
        assert!(row_of(&rows, "Issues (2)") < row_of(&rows, "lookup.rs:1 lookup and"));
        assert!(row_of(&rows, "cache.rs:1 the test") < row_of(&rows, "Nits (1)"));
        assert!(row_of(&rows, "Nits (1)") < row_of(&rows, "Questions (1)"));

        let on = |state: &AppState| {
            let guided = state.guided.as_deref().unwrap();
            let step = guided.step().unwrap();
            (step.path.clone(), step.lines[guided.cursor].text.clone())
        };
        // The cache concern, lookup.rs in it, comes before the tests.
        press(&mut state, Right);
        assert_eq!(
            on(&state),
            ("src/lookup.rs".into(), "+lookup calls the cache".into())
        );
        press(&mut state, Right);
        assert_eq!(on(&state).0, "tests/cache.rs");

        press(&mut state, Char('t'));
        let guided = state.guided.as_deref().unwrap();
        assert!(
            guided
                .open_findings()
                .iter()
                .all(|open| open.finding.text != "the test never fills the cache.")
        );
        press(&mut state, Right);
        assert_eq!(
            on(&state).0,
            "src/cache.rs",
            "the nit, once the issues are done"
        );
    }

    #[test]
    fn a_click_on_a_finding_opens_its_file_on_its_line() {
        let mut state = walk_with_findings_read();
        let (list, _) = panes(&state);
        let rows = list_text(&state);
        let row = list.y + row_of(&rows, "cache.rs:1 the test") as u16;

        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            list.x + 6,
            row,
        );

        let guided = state.guided.as_deref().unwrap();
        let step = guided.step().unwrap();
        assert_eq!(step.path, "tests/cache.rs");
        assert_eq!(step.lines[guided.cursor].text, "+evicts the oldest");
    }

    #[test]
    fn the_plan_shows_how_far_it_has_got() {
        let mut state = walk_with_plan();
        let guided = state.guided.as_mut().unwrap();
        guided
            .commentary
            .insert(OVERVIEW_KEY.to_string(), Commentary::default());
        assert!(matches!(guided.planning(), Some(Progress::Reading { .. })));

        guided.commentary.get_mut(OVERVIEW_KEY).unwrap().text =
            "Summary: Adds a cache.\nConcern: The cache\nFiles: src/cache.rs, src/lookup.rs\n"
                .into();
        assert_eq!(
            guided.planning(),
            Some(Progress::Placing { named: 2, total: 4 })
        );
        assert!(
            screen(&state)
                .iter()
                .any(|row| row.contains("2/4 files placed"))
        );
    }

    /// Scrolling the list opens nothing, so nothing is read; a click opens
    /// the row under it, and the list stays where it was scrolled.
    #[test]
    fn the_wheel_scrolls_the_step_list_and_only_a_click_opens_a_step() {
        let mut state = walk_on_a_long_hunk();
        let diff: String = (0..60)
            .map(|n| {
                format!("diff --git a/f{n:02}.rs b/f{n:02}.rs\n--- a/f{n:02}.rs\n+++ b/f{n:02}.rs\n@@ -1 +1 @@\n-a\n+b\n")
            })
            .collect();
        let guided = state.guided.as_mut().unwrap();
        guided.steps = crate::git::guided::parse_files(&diff);
        guided.current = OVERVIEW;
        let (list, _) = panes(&state);
        let wheel =
            |state: &mut AppState| mouse(state, MouseEventKind::ScrollDown, list.x + 2, list.y + 2);

        for _ in 0..4 {
            wheel(&mut state);
        }
        assert_eq!(state.guided.as_ref().unwrap().current, OVERVIEW);
        assert!(state.guided.as_ref().unwrap().asks.is_empty());
        let rows = list_text(&state);
        assert!(
            !rows[0].contains("Overview"),
            "the list scrolled: {rows:#?}"
        );
        let row = row_of(&rows, "f20.rs");

        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            list.x + 4,
            list.y + row as u16,
        );

        let guided = state.guided.as_deref().unwrap();
        assert_eq!(guided.step().map(|s| s.path.as_str()), Some("f20.rs"));
        assert_eq!(
            row_of(&list_text(&state), "f20.rs"),
            row,
            "the list stays put"
        );
    }

    /// Going to the final review shows it working from the first moment: in
    /// line behind the reads already running, then reading the change, then
    /// how many findings it has written.
    #[test]
    fn the_final_review_shows_how_far_it_has_got() {
        let mut state = walk_with_plan();
        state.ai_assist = true;
        let guided = state.guided.as_mut().unwrap();
        guided.current = guided.final_step();
        let (_busy, rx) = std::sync::mpsc::channel();
        guided.asks.push(Ask {
            target: Target::Commentary(guided.steps[0].key()),
            rx,
        });
        assert_eq!(guided.final_progress(), Some(Progress::Queued { ahead: 1 }));
        assert!(
            screen(&state)
                .iter()
                .any(|row| row.contains("queued behind 1 read"))
        );

        let guided = state.guided.as_mut().unwrap();
        let key = guided.final_key();
        guided.commentary.insert(
            key.clone(),
            Commentary {
                started: Some(std::time::Instant::now()),
                prompt_bytes: 50_000,
                ..Default::default()
            },
        );
        assert!(matches!(
            guided.final_progress(),
            Some(Progress::Reading { .. })
        ));
        assert!(
            screen(&state)
                .iter()
                .any(|row| row.contains("reading the whole change"))
        );

        let guided = state.guided.as_mut().unwrap();
        guided.commentary.get_mut(&key).unwrap().text =
            "Summary: Two parts.\nsrc/cache.rs:L1 issue: evict is never called.\n".into();
        assert_eq!(
            guided.final_progress(),
            Some(Progress::Finding { so_far: 1 })
        );
        assert!(
            screen(&state)
                .iter()
                .any(|row| row.contains("1 finding(s) so far"))
        );

        let guided = state.guided.as_mut().unwrap();
        guided.commentary.get_mut(&key).unwrap().done = true;
        assert_eq!(guided.final_progress(), None);
    }
}
