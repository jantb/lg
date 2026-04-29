use anyhow::{Context, Result};
use chrono::Utc;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{
    io::Stdout,
    path::{Component, Path},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use crate::{
    config::{
        BACKGROUND_FETCH_INTERVAL_SECS, COMMIT_LIST_LIMIT, DEFAULT_PUSH_REMOTE,
        STATUS_MSG_LIFETIME_SECS, TICK_MS,
    },
    panel,
    state::{
        AppState, CheckoutJob, CheckoutMsg, ConflictFollowup, DiffJob, DiffMsg, DiffSource,
        FetchJob, FetchMsg, FlowAction, GenMsg, Modal, OperationJob, OperationKind, OperationMsg,
        Pane, PendingAction, PushJob, PushMsg, RefreshJob, RefreshMsg, RefreshSnapshot, TreeKind,
        WorkflowJob, WorkflowMsg,
    },
    ui,
};

pub struct App {
    pub state: AppState,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    file_events: Receiver<notify::Result<notify::Event>>,
    _file_watcher: RecommendedWatcher,
    last_fetch_started: Instant,
}

/// Headless app backed by a generic [`Backend`]; used by tests and the harness.
pub struct HeadlessApp<B: Backend> {
    pub state: AppState,
    pub terminal: Terminal<B>,
}

// ─── free helpers ────────────────────────────────────────────────────────────

fn next_pane(p: Pane) -> Pane {
    match p {
        Pane::Status => Pane::Files,
        Pane::Files => Pane::Branches,
        Pane::Branches => Pane::Commits,
        Pane::Commits => Pane::Main,
        Pane::Main => Pane::Status,
    }
}

fn prev_pane(p: Pane) -> Pane {
    match p {
        Pane::Status => Pane::Main,
        Pane::Files => Pane::Status,
        Pane::Branches => Pane::Files,
        Pane::Commits => Pane::Branches,
        Pane::Main => Pane::Commits,
    }
}

fn first_status_line(s: &str) -> String {
    s.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(s)
        .chars()
        .take(120)
        .collect()
}

fn git_job_running(state: &AppState) -> bool {
    state.push_job.is_some()
        || state.checkout_job.is_some()
        || state.operation_job.is_some()
        || state.fetch_job.is_some()
        || state.workflow_job.is_some()
}

fn selected_diff_source(state: &AppState) -> DiffSource {
    match state.focus {
        Pane::Files => {
            let rows = state.tree_rows();
            match rows.get(state.files_idx) {
                Some(row) => match &row.kind {
                    TreeKind::AllChanges => DiffSource::All,
                    TreeKind::Folder { .. } => DiffSource::Folder(row.path.clone()),
                    TreeKind::File { entry_idx } => state
                        .files
                        .get(*entry_idx)
                        .map(|f| DiffSource::File(f.path.clone()))
                        .unwrap_or(DiffSource::None),
                },
                None => DiffSource::None,
            }
        }
        Pane::Commits => state
            .commits
            .get(state.commits_idx)
            .map(|c| DiffSource::Commit(c.sha.clone()))
            .unwrap_or(DiffSource::None),
        Pane::Branches => state
            .branches
            .get(state.branches_idx)
            .map(|b| DiffSource::Branch(b.name.clone()))
            .unwrap_or(DiffSource::None),
        _ => DiffSource::None,
    }
}

fn load_diff_text(source: &DiffSource) -> String {
    match source {
        DiffSource::None | DiffSource::Branch(_) => String::new(),
        DiffSource::All => crate::git::all_diffs().unwrap_or_else(|e| format!("error: {e}")),
        DiffSource::File(path) => {
            crate::git::file_diff(path).unwrap_or_else(|e| format!("error: {e}"))
        }
        DiffSource::Folder(path) => {
            crate::git::folder_diff(path).unwrap_or_else(|e| format!("error: {e}"))
        }
        DiffSource::Commit(sha) => {
            crate::git::show_commit(sha).unwrap_or_else(|e| format!("error: {e}"))
        }
    }
}

fn build_refresh_snapshot() -> RefreshSnapshot {
    let mut errors = Vec::new();
    let files = match crate::git::status_entries() {
        Ok(files) => Some(files),
        Err(e) => {
            errors.push(format!("git status failed: {e}"));
            None
        }
    };
    let branches = match crate::git::list_branches() {
        Ok(branches) => Some(branches),
        Err(e) => {
            errors.push(format!("git branch failed: {e}"));
            None
        }
    };
    let commits = match crate::git::list_commits(COMMIT_LIST_LIMIT) {
        Ok(commits) => Some(commits),
        Err(e) => {
            errors.push(format!("git log failed: {e}"));
            None
        }
    };
    let unpushed_shas = match crate::git::unpushed_shas() {
        Ok(shas) => Some(shas),
        Err(e) => {
            errors.push(format!("unpushed check failed: {e}"));
            None
        }
    };
    let branch = crate::git::head_branch().ok().or_else(|| {
        branches.as_ref().and_then(|branches| {
            branches
                .iter()
                .find(|branch| branch.is_current)
                .map(|branch| branch.name.clone())
        })
    });
    let current_branch_releases = branch
        .as_deref()
        .and_then(|branch| crate::git::branch_release_status(branch).ok())
        .unwrap_or_default();

    RefreshSnapshot {
        files,
        branches,
        flow_branches_available: crate::git::flow_branches_available(),
        commits,
        unpushed_shas,
        branch,
        current_branch_releases,
        remote_url: crate::git::remote_url(DEFAULT_PUSH_REMOTE).ok(),
        ahead_behind: crate::git::counts_ahead_behind().ok(),
        errors,
    }
}

fn prime_branches(state: &mut AppState) {
    if let Ok(branches) = crate::git::list_branches() {
        state.branch = branches
            .iter()
            .find(|branch| branch.is_current)
            .map(|branch| branch.name.clone());
        state.branches = branches;
        state.clamp();
    }
}

fn path_has_ignored_component(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| matches!(name, ".git" | "target")),
        _ => false,
    })
}

fn should_refresh_for_fs_event(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    if event.paths.is_empty() {
        return true;
    }
    event
        .paths
        .iter()
        .any(|path| !path_has_ignored_component(path))
}

fn watch_current_dir() -> Result<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("start file watcher")?;

    let cwd = std::env::current_dir().context("resolve current directory")?;
    watcher
        .watch(&cwd, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", cwd.display()))?;

    Ok((watcher, rx))
}

fn spawn_push(state: &mut AppState) {
    if git_job_running(state) {
        return;
    }
    let branch = state.branch.clone().unwrap_or_default();
    let remote = DEFAULT_PUSH_REMOTE.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let tbranch = branch.clone();
    let tremote = remote.clone();
    std::thread::spawn(move || match crate::git::push(&tremote, &tbranch) {
        Ok(out) => {
            let line = out
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("pushed")
                .to_owned();
            let _ = tx.send(PushMsg::Done(line));
        }
        Err(e) => {
            let _ = tx.send(PushMsg::Error(e.to_string()));
        }
    });
    state.push_job = Some(PushJob {
        rx,
        spinner: 0,
        branch,
        remote,
    });
    state.set_status("pushing\u{2026}", false);
}

pub(crate) fn checkout_branch_async(state: &mut AppState, branch: String) {
    if git_job_running(state) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let target = branch.clone();
    std::thread::spawn(move || match crate::git::checkout_branch(&target) {
        Ok(out) => {
            let line = out
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("checked out")
                .to_owned();
            let _ = tx.send(CheckoutMsg::Done(line));
        }
        Err(e) => {
            let _ = tx.send(CheckoutMsg::Error(e.to_string()));
        }
    });
    state.checkout_job = Some(CheckoutJob {
        rx,
        spinner: 0,
        branch: branch.clone(),
    });
    state.set_status(format!("checking out {branch}\u{2026}"), false);
}

fn spawn_operation<F>(state: &mut AppState, label: &'static str, kind: OperationKind, work: F)
where
    F: FnOnce() -> Result<String> + Send + 'static,
{
    if git_job_running(state) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || match work() {
        Ok(s) => {
            let _ = tx.send(OperationMsg::Done(s));
        }
        Err(e) => {
            let _ = tx.send(OperationMsg::Error(e.to_string()));
        }
    });
    state.operation_job = Some(OperationJob {
        rx,
        spinner: 0,
        label,
        kind,
    });
    state.set_status(format!("{label}\u{2026}"), false);
}

pub(crate) fn run_flow_action(state: &mut AppState, action: FlowAction, input: Option<String>) {
    if git_job_running(state) {
        return;
    }
    if !state.flow_available() {
        state.modal = Modal::None;
        state.set_status("flow unavailable: missing develop or release/next", true);
        return;
    }

    let current = state.branch.clone().unwrap_or_default();
    let label = action.label().to_owned();
    let steps = workflow_steps(action, &current, input.as_deref());
    let thread_steps = steps.clone();
    state.conflict_followup = conflict_followup_for_flow(action, &current);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut step_idx = 0usize;
        let mut progress = || {
            let _ = tx.send(WorkflowMsg::Progress(step_idx));
            step_idx += 1;
        };
        let res = match action {
            FlowAction::MergeMain => {
                crate::git::flow_merge_main_into_current_with_progress(&current, &mut progress)
            }
            FlowAction::ReleaseDev => crate::git::flow_release_current_with_progress(
                &current,
                crate::config::BRANCH_DEV,
                &mut progress,
            ),
            FlowAction::ReleaseTest => crate::git::flow_release_current_with_progress(
                &current,
                crate::config::BRANCH_TEST,
                &mut progress,
            ),
            FlowAction::ResetDev => crate::git::flow_reset_branch_from_main_with_progress(
                &current,
                crate::config::BRANCH_DEV,
                &mut progress,
            ),
            FlowAction::ResetTest => crate::git::flow_reset_branch_from_main_with_progress(
                &current,
                crate::config::BRANCH_TEST,
                &mut progress,
            ),
            FlowAction::NewFeature => {
                for _ in &thread_steps {
                    progress();
                }
                crate::git::flow_create_feature_branch(&current, &input.unwrap_or_default())
            }
            FlowAction::CleanOrphans => {
                for _ in &thread_steps {
                    progress();
                }
                crate::git::flow_clean_orphan_branches(&current)
            }
        };
        match res {
            Ok(s) => {
                let _ = tx.send(WorkflowMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(WorkflowMsg::Error(e.to_string()));
            }
        }
    });

    state.workflow_job = Some(WorkflowJob {
        rx,
        spinner: 0,
        label,
        steps,
        current_step: None,
    });
    state.set_status("running flow workflow\u{2026}", false);
}

fn conflict_followup_for_flow(action: FlowAction, current: &str) -> Option<ConflictFollowup> {
    match action {
        FlowAction::MergeMain => Some(ConflictFollowup {
            push_branch: Some(current.to_string()),
            return_branch: Some(current.to_string()),
        }),
        FlowAction::ReleaseDev => Some(ConflictFollowup {
            push_branch: Some(crate::config::BRANCH_DEV.to_string()),
            return_branch: Some(current.to_string()),
        }),
        FlowAction::ReleaseTest => Some(ConflictFollowup {
            push_branch: Some(crate::config::BRANCH_TEST.to_string()),
            return_branch: Some(current.to_string()),
        }),
        FlowAction::ResetDev
        | FlowAction::ResetTest
        | FlowAction::NewFeature
        | FlowAction::CleanOrphans => None,
    }
}

fn workflow_steps(action: FlowAction, current: &str, input: Option<&str>) -> Vec<String> {
    match action {
        FlowAction::MergeMain => vec![
            "stash current changes".into(),
            "create safety backup".into(),
            "fetch origin".into(),
            "checkout main".into(),
            "update main from origin/main".into(),
            format!("checkout {current}"),
            format!("merge origin/{} into {current}", crate::config::BRANCH_MAIN),
            format!("push {current}"),
            "restore stashed changes".into(),
        ],
        FlowAction::ReleaseDev => release_steps(current, crate::config::BRANCH_DEV),
        FlowAction::ReleaseTest => release_steps(current, crate::config::BRANCH_TEST),
        FlowAction::ResetDev => reset_steps(current, crate::config::BRANCH_DEV),
        FlowAction::ResetTest => reset_steps(current, crate::config::BRANCH_TEST),
        FlowAction::NewFeature => vec![format!(
            "create {}",
            input.filter(|s| !s.is_empty()).unwrap_or("new branch")
        )],
        FlowAction::CleanOrphans => vec!["scan branches".into(), "delete orphan branches".into()],
    }
}

fn release_steps(current: &str, target: &str) -> Vec<String> {
    vec![
        "stash current changes".into(),
        "create safety backup".into(),
        format!("push {current}"),
        "fetch origin".into(),
        format!("sync {target} from origin/{target}"),
        format!("checkout {target}"),
        format!("merge origin/{}", crate::config::BRANCH_MAIN),
        format!("merge origin/{current}"),
        format!("push HEAD to origin/{target}"),
        format!("checkout {current}"),
        "restore stashed changes".into(),
    ]
}

fn reset_steps(current: &str, target: &str) -> Vec<String> {
    let mut steps = vec!["fetch origin".into()];
    if current != target {
        steps.push(format!("checkout {target}"));
    }
    steps.extend([
        "create safety backup".into(),
        format!("reset {target} to origin/{}", crate::config::BRANCH_MAIN),
        format!("force push {target}"),
    ]);
    if current != target {
        steps.push(format!("checkout {current}"));
    }
    steps
}

pub(crate) fn run_conflict_llm(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let conflicts = state.conflicts.clone();
    let log = state.conflict_log.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let res = crate::git::conflict_bundle(&conflicts, &log)
            .and_then(crate::ollama::resolve_merge_conflicts);
        match res {
            Ok(patch) => {
                let _ = tx.send(WorkflowMsg::Patch(patch));
            }
            Err(e) => {
                let _ = tx.send(WorkflowMsg::Error(e.to_string()));
            }
        }
    });
    state.workflow_job = Some(WorkflowJob {
        rx,
        spinner: 0,
        label: "LLM conflict resolution".to_string(),
        steps: Vec::new(),
        current_step: None,
    });
    state.set_status("asking LLM to resolve conflicts\u{2026}", false);
}

pub(crate) fn apply_pending_llm_patch(state: &mut AppState) {
    let Some(patch) = state.pending_llm_patch.clone() else {
        state.set_status("no LLM patch to apply", true);
        return;
    };
    match crate::git::apply_patch_text(&patch) {
        Ok(()) => {
            state.pending_llm_patch = None;
            state.conflict_log =
                "LLM patch applied. Review, then press c to stage resolved files and continue."
                    .to_string();
            state.set_status("LLM patch applied", false);
        }
        Err(e) => {
            state.conflict_log = e.to_string();
            state.set_status("LLM patch failed", true);
        }
    }
}

pub(crate) fn run_conflict_validation(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || match crate::git::run_detected_validation() {
        Ok(log) => {
            let _ = tx.send(WorkflowMsg::Done(log));
        }
        Err(e) => {
            let _ = tx.send(WorkflowMsg::Error(e.to_string()));
        }
    });
    state.workflow_job = Some(WorkflowJob {
        rx,
        spinner: 0,
        label: "validation".to_string(),
        steps: Vec::new(),
        current_step: None,
    });
    state.set_status("running validation\u{2026}", false);
}

pub(crate) fn continue_conflict_operation(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let followup = state.conflict_followup.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        match crate::git::continue_in_progress_operation_with_followup(
            followup.as_ref().and_then(|f| f.push_branch.as_deref()),
            followup.as_ref().and_then(|f| f.return_branch.as_deref()),
        ) {
            Ok(s) => {
                let _ = tx.send(WorkflowMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(WorkflowMsg::Error(e.to_string()));
            }
        }
    });
    state.workflow_job = Some(WorkflowJob {
        rx,
        spinner: 0,
        label: "stage resolved files and continue".to_string(),
        steps: vec![
            "stage resolved files".to_string(),
            "continue Git operation".to_string(),
            "push release branch if needed".to_string(),
            "return to feature branch if needed".to_string(),
        ],
        current_step: None,
    });
    state.set_status("staging resolved files and continuing\u{2026}", false);
}

pub(crate) fn abort_conflict_operation(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let return_branch = state
        .conflict_followup
        .as_ref()
        .and_then(|f| f.return_branch.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        match crate::git::abort_in_progress_operation_with_return(return_branch.as_deref()) {
            Ok(s) => {
                let _ = tx.send(WorkflowMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(WorkflowMsg::Error(e.to_string()));
            }
        }
    });
    state.workflow_job = Some(WorkflowJob {
        rx,
        spinner: 0,
        label: "abort merge".to_string(),
        steps: Vec::new(),
        current_step: None,
    });
    state.set_status("aborting git operation\u{2026}", false);
}

fn footer_spec(pane: Pane) -> (u8, &'static str, &'static [(&'static str, &'static str)]) {
    match pane {
        Pane::Status => (1, "Status", &[("F", "flow"), ("?", "help"), ("q", "quit")]),
        Pane::Files => (
            2,
            "Files",
            &[
                ("space", "stage"),
                ("u", "unstage"),
                ("A/U", "all"),
                ("c", "commit"),
                ("p/P", "push"),
                ("F", "flow"),
                ("?", "help"),
            ],
        ),
        Pane::Branches => (
            3,
            "Branches",
            &[("Enter", "checkout"), ("F", "flow"), ("?", "help")],
        ),
        Pane::Commits => (
            4,
            "Commits",
            &[
                ("j/k", "navigate"),
                ("Enter", "focus diff"),
                ("F", "flow"),
                ("?", "help"),
            ],
        ),
        Pane::Main => (
            0,
            "Diff",
            &[
                ("j/k", "scroll"),
                ("g/G", "top/bot"),
                ("F", "flow"),
                ("?", "help"),
            ],
        ),
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    // Horizontal split: left flexible, right status area.
    let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(40)]).split(area);

    // Left: modal-aware spec.
    let left_spans: Vec<Span> = match state.modal {
        Modal::None => {
            let (n, name, pairs) = footer_spec(state.focus);
            let mut spans = vec![Span::styled(
                format!("[{n}] {name} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            for (i, (key, label)) in pairs.iter().enumerate() {
                if *key == "F" && !state.flow_available() {
                    continue;
                }
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if pairs
                    .iter()
                    .skip(i + 1)
                    .any(|(next_key, _)| *next_key != "F" || state.flow_available())
                {
                    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        }
        Modal::Commit => {
            let pairs: &[(&str, &str)] = &[
                ("Ctrl+S", "commit"),
                ("Enter", "newline"),
                ("Ctrl+R", "regen"),
                ("Esc", "cancel"),
            ];
            let mut spans = vec![Span::styled(
                "Commit modal ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            for (i, (key, label)) in pairs.iter().enumerate() {
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if i + 1 < pairs.len() {
                    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        }
        Modal::Push => {
            let pairs: &[(&str, &str)] = &[("Enter", "push"), ("Esc", "cancel")];
            let mut spans = vec![Span::styled(
                "Push modal ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            for (i, (key, label)) in pairs.iter().enumerate() {
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if i + 1 < pairs.len() {
                    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        }
        Modal::Help => {
            let pairs: &[(&str, &str)] = &[("any key", "close")];
            let mut spans = vec![Span::styled(
                "Help ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            for (i, (key, label)) in pairs.iter().enumerate() {
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if i + 1 < pairs.len() {
                    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        }
        Modal::Flow => {
            let pairs: &[(&str, &str)] = if state.flow_available() {
                &[("j/k", "select"), ("Enter", "continue"), ("Esc", "back")]
            } else {
                &[("Esc", "back")]
            };
            let mut spans = vec![Span::styled(
                "Flow ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            for (i, (key, label)) in pairs.iter().enumerate() {
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if i + 1 < pairs.len() {
                    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        }
        Modal::Conflict => {
            let pairs: &[(&str, &str)] = &[
                ("l", "llm"),
                ("v", "validate"),
                ("c", "continue"),
                ("a", "abort"),
            ];
            let mut spans = vec![Span::styled(
                "Conflict ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )];
            for (i, (key, label)) in pairs.iter().enumerate() {
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if i + 1 < pairs.len() {
                    spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        }
    };

    frame.render_widget(
        Paragraph::new(Line::from(left_spans)).alignment(Alignment::Left),
        chunks[0],
    );

    // Right: live status or branch name.
    let (right_text, right_color) = match (&state.status, state.activity_label()) {
        (Some(s), Some(label)) if !s.is_error => {
            let spinner = crate::state::SPINNER_FRAMES
                [state.animation_tick % crate::state::SPINNER_FRAMES.len()];
            let text = if s.text.starts_with(label) {
                format!("{spinner} {}", s.text)
            } else {
                format!("{spinner} {label}: {}", s.text)
            };
            (text, Color::Cyan)
        }
        (Some(s), _) => {
            let icon = if s.is_error { "\u{2717}" } else { "\u{2713}" };
            (
                format!("{icon} {}", s.text),
                if s.is_error { Color::Red } else { Color::Green },
            )
        }
        (None, Some(label)) => {
            let spinner = crate::state::SPINNER_FRAMES
                [state.animation_tick % crate::state::SPINNER_FRAMES.len()];
            (format!("{spinner} {label}\u{2026}"), Color::Cyan)
        }
        (None, None) => (
            format!(
                "\u{2022} {}",
                state.branch.as_deref().unwrap_or("no branch")
            ),
            Color::DarkGray,
        ),
    };
    frame.render_widget(
        Paragraph::new(Span::styled(right_text, Style::default().fg(right_color)))
            .alignment(Alignment::Right),
        chunks[1],
    );
}

// ─── HeadlessApp ─────────────────────────────────────────────────────────────

impl<B: Backend> HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    pub fn new(backend: B) -> Result<Self> {
        let terminal = Terminal::new(backend).context("create headless terminal")?;
        Ok(Self {
            state: AppState::new(),
            terminal,
        })
    }

    pub fn render(&mut self) -> Result<()> {
        self.state.advance_animation();

        // Compute viewport height before the draw closure so we can update state.
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects_pre = ui::split_layout_with_environments(area, self.state.flow_available());
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout_with_environments(area, state.flow_available());
            let focused_pane = state.focus;

            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
            panel::environments::render(state, rects.environments, frame);
            panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
            panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
            panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
            panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

            draw_footer(frame, rects.footer, state);

            match state.modal {
                Modal::None => {}
                Modal::Commit => panel::commit::render(state, area, frame),
                Modal::Push => panel::push::render(state, area, frame),
                Modal::Help => panel::help::render(state, area, frame),
                Modal::Flow => panel::flow::render(state, area, frame),
                Modal::Conflict => panel::conflict::render(state, area, frame),
            }
        })?;
        Ok(())
    }

    pub fn send_key(&mut self, k: KeyEvent) -> Result<()> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return self.render();
        }
        match self.state.modal {
            Modal::Help => {
                panel::help::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Commit => {
                panel::commit::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Push => {
                panel::push::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Flow => {
                panel::flow::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Conflict => {
                panel::conflict::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::None => {}
        }
        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
            }
            KeyCode::Char('F') => {
                if self.state.flow_available() {
                    self.state.modal = Modal::Flow;
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.state.should_quit = true;
            }
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
            }
            KeyCode::Char('c') => {
                self.state.open_commit_modal();
            }
            KeyCode::Char('p') => {
                self.state.modal = Modal::Push;
            }
            KeyCode::Char('P') => {
                if self.state.unpushed_shas.is_empty() {
                    self.state.set_status("nothing to push", false);
                } else {
                    spawn_push(&mut self.state);
                }
            }
            _ => match self.state.focus {
                Pane::Status => {}
                Pane::Files => panel::files::handle_key(&mut self.state, k)?,
                Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
                Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
                Pane::Main => panel::main::handle_key(&mut self.state, k)?,
            },
        }
        self.render()
    }
}

// ─── App ─────────────────────────────────────────────────────────────────────

impl App {
    pub fn new() -> Result<Self> {
        if !crate::git::is_repo() {
            anyhow::bail!("not a git repository (or any parent up to mount point)");
        }

        let (_file_watcher, file_events) = watch_current_dir()?;

        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
            prev_hook(info);
        }));

        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context("enter alt screen")?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("create terminal")?;

        let mut app = Self {
            state: AppState::new(),
            terminal,
            file_events,
            _file_watcher,
            last_fetch_started: Instant::now()
                - Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS),
        };
        prime_branches(&mut app.state);
        app.start_refresh(true);
        app.start_fetch();
        Ok(app)
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.state.should_quit {
                break;
            }

            self.render()?;

            self.drain_generation();
            self.drain_push_job()?;
            self.drain_checkout_job()?;
            self.drain_operation_job()?;
            self.drain_fetch_job();
            self.drain_refresh_job();
            self.drain_diff_job();
            self.drain_workflow_job()?;
            self.drain_file_events()?;
            self.maybe_start_periodic_fetch();

            let poll_ms = if self.state.generation.is_some()
                || self.state.push_job.is_some()
                || self.state.checkout_job.is_some()
                || self.state.operation_job.is_some()
                || self.state.fetch_job.is_some()
                || self.state.refresh_job.is_some()
                || self.state.diff_job.is_some()
                || self.state.workflow_job.is_some()
            {
                80
            } else {
                TICK_MS
            };
            if event::poll(Duration::from_millis(poll_ms))? {
                match event::read()? {
                    Event::Key(k) => self.handle_key(k)?,
                    Event::Mouse(m) => self.handle_mouse(m)?,
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }

            // Dispatch pending IO action.
            if let Some(action) = self.state.pending_action.take() {
                match action {
                    PendingAction::GenerateMessage => match crate::git::staged_diff() {
                        Ok(diff) => {
                            let (tx, rx) = std::sync::mpsc::channel();
                            std::thread::spawn(move || {
                                crate::ollama::stream_commit_message(diff, tx);
                            });
                            self.state.start_generation(rx);
                            self.state.set_status("generating\u{2026}", false);
                        }
                        Err(e) => {
                            self.state.set_status(e.to_string(), true);
                        }
                    },
                    PendingAction::Commit => {
                        let msg = self.state.commit_message.clone();
                        spawn_operation(
                            &mut self.state,
                            "committing",
                            OperationKind::Commit,
                            move || {
                                let out = crate::git::commit(&msg)?;
                                Ok(out.lines().next().unwrap_or("committed").to_owned())
                            },
                        );
                    }
                    PendingAction::Push => {
                        spawn_push(&mut self.state);
                    }
                    PendingAction::StageAll => {
                        spawn_operation(
                            &mut self.state,
                            "staging",
                            OperationKind::Worktree,
                            || {
                                crate::git::stage_all()?;
                                Ok("staged all".to_string())
                            },
                        );
                    }
                    PendingAction::UnstageAll => {
                        spawn_operation(
                            &mut self.state,
                            "unstaging",
                            OperationKind::Worktree,
                            || {
                                crate::git::unstage_all()?;
                                Ok("unstaged all".to_string())
                            },
                        );
                    }
                    PendingAction::StagePath(path) => {
                        spawn_operation(
                            &mut self.state,
                            "staging",
                            OperationKind::Worktree,
                            move || {
                                crate::git::stage(&path)?;
                                Ok(format!("staged {path}"))
                            },
                        );
                    }
                    PendingAction::UnstagePath(path) => {
                        spawn_operation(
                            &mut self.state,
                            "unstaging",
                            OperationKind::Worktree,
                            move || {
                                crate::git::unstage(&path)?;
                                Ok(format!("unstaged {path}"))
                            },
                        );
                    }
                }
            }

            // Expire stale status messages.
            if let Some(ref s) = self.state.status.clone() {
                if (Utc::now() - s.at).num_seconds() >= STATUS_MSG_LIFETIME_SECS {
                    self.state.status = None;
                }
            }
        }
        Ok(())
    }

    fn start_refresh(&mut self, refresh_diff: bool) {
        self.start_refresh_with_status(refresh_diff, true);
    }

    fn start_refresh_with_status(&mut self, refresh_diff: bool, show_status: bool) {
        if let Some(job) = self.state.refresh_job.as_mut() {
            job.refresh_diff |= refresh_diff;
            self.state.refresh_pending = true;
            self.state.refresh_pending_diff |= refresh_diff;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(RefreshMsg::Done(Box::new(build_refresh_snapshot())));
        });
        self.state.refresh_job = Some(RefreshJob {
            rx,
            spinner: 0,
            refresh_diff,
        });
        if show_status {
            self.state.set_status("refreshing\u{2026}", false);
        }
    }

    fn start_fetch(&mut self) {
        if git_job_running(&self.state) {
            return;
        }
        self.last_fetch_started = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || match crate::git::fetch_updates() {
            Ok(s) => {
                let _ = tx.send(FetchMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(FetchMsg::Error(e.to_string()));
            }
        });
        self.state.fetch_job = Some(FetchJob { rx, spinner: 0 });
    }

    fn maybe_start_periodic_fetch(&mut self) {
        if self.last_fetch_started.elapsed() >= Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS)
        {
            self.start_fetch();
        }
    }

    fn start_diff_job(&mut self, force: bool) {
        let source = selected_diff_source(&self.state);
        if !force && source == self.state.diff_source {
            return;
        }
        if self
            .state
            .diff_job
            .as_ref()
            .is_some_and(|job| job.source == source)
        {
            return;
        }
        self.state.diff_source = source.clone();
        self.state.diff_offset = 0;
        self.state.diff_text = if matches!(source, DiffSource::None | DiffSource::Branch(_)) {
            String::new()
        } else {
            "loading diff...".to_string()
        };
        self.state.diff_line_count =
            self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
        if matches!(source, DiffSource::None | DiffSource::Branch(_)) {
            self.state.diff_job = None;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_source = source.clone();
        std::thread::spawn(move || {
            let text = load_diff_text(&thread_source);
            let _ = tx.send(DiffMsg::Done {
                source: thread_source,
                text,
            });
        });
        self.state.diff_job = Some(DiffJob {
            rx,
            spinner: 0,
            source,
        });
    }

    fn drain_file_events(&mut self) -> Result<()> {
        let mut should_refresh = false;
        while let Ok(event) = self.file_events.try_recv() {
            match event {
                Ok(event) => {
                    if should_refresh_for_fs_event(&event) {
                        should_refresh = true;
                    }
                }
                Err(err) => {
                    self.state
                        .set_status(format!("file watch failed: {err}"), true);
                }
            }
        }
        if should_refresh {
            self.start_refresh(true);
        }
        Ok(())
    }

    fn apply_refresh_snapshot(&mut self, snapshot: RefreshSnapshot, refresh_diff: bool) {
        if let Some(files) = snapshot.files {
            self.state.files = files;
        }
        if let Some(branches) = snapshot.branches {
            self.state.branches = branches;
        }
        self.state.flow_branches_available = snapshot.flow_branches_available;
        if let Some(commits) = snapshot.commits {
            self.state.commits = commits;
        }
        if let Some(shas) = snapshot.unpushed_shas {
            self.state.unpushed_shas = shas;
        }
        self.state.branch = snapshot.branch;
        self.state.current_branch_releases = snapshot.current_branch_releases;
        self.state.remote_url = snapshot.remote_url;
        self.state.ahead_behind = snapshot.ahead_behind;
        if let Some(error) = snapshot.errors.into_iter().next() {
            self.state.set_status(error, true);
        }
        self.state.clamp();
        if refresh_diff {
            self.start_diff_job(true);
        }
    }

    fn drain_refresh_job(&mut self) {
        let mut finished = None;
        if let Some(job) = self.state.refresh_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                let RefreshMsg::Done(snapshot) = msg;
                finished = Some((*snapshot, job.refresh_diff));
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some((snapshot, refresh_diff)) = finished {
            let pending_refresh = self.state.refresh_pending;
            let pending_diff = self.state.refresh_pending_diff;
            self.state.refresh_job = None;
            self.state.refresh_pending = false;
            self.state.refresh_pending_diff = false;
            self.apply_refresh_snapshot(snapshot, refresh_diff);
            if pending_refresh {
                self.start_refresh(pending_diff);
            }
        }
    }

    fn drain_diff_job(&mut self) {
        let mut finished = None;
        if let Some(job) = self.state.diff_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                let DiffMsg::Done { source, text } = msg;
                finished = Some((source, text));
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some((source, text)) = finished {
            self.state.diff_job = None;
            if source == self.state.diff_source {
                self.state.diff_text = text;
                self.state.diff_line_count =
                    self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
            }
        }
    }

    fn drain_fetch_job(&mut self) {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.fetch_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    FetchMsg::Done(s) => finished = Some(Ok(s)),
                    FetchMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.fetch_job = None;
            match res {
                Ok(s) if s != "no remotes configured" => self.state.set_status(s, false),
                Ok(_) => {}
                Err(e) => self.state.set_status(first_status_line(&e), true),
            }
            self.start_refresh_with_status(false, false);
        }
    }

    fn drain_push_job(&mut self) -> Result<()> {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.push_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    PushMsg::Done(s) => finished = Some(Ok(s)),
                    PushMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.push_job = None;
            self.state.modal = Modal::None;
            match res {
                Ok(s) => self.state.set_status(s, false),
                Err(e) => self.state.set_status(e, true),
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_checkout_job(&mut self) -> Result<()> {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.checkout_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    CheckoutMsg::Done(s) => finished = Some(Ok(s)),
                    CheckoutMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.checkout_job = None;
            match res {
                Ok(s) => self.state.set_status(s, false),
                Err(e) => self.state.set_status(e, true),
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_operation_job(&mut self) -> Result<()> {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.operation_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    OperationMsg::Done(s) => finished = Some(Ok(s)),
                    OperationMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            let kind = self
                .state
                .operation_job
                .as_ref()
                .map(|job| job.kind)
                .unwrap_or(OperationKind::Worktree);
            self.state.operation_job = None;
            match res {
                Ok(s) => {
                    self.state.set_status(s, false);
                    if kind == OperationKind::Commit {
                        self.state.modal = Modal::None;
                        self.state.commit_message.clear();
                    }
                }
                Err(e) => self.state.set_status(e, true),
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_workflow_job(&mut self) -> Result<()> {
        let mut finished: Option<WorkflowMsg> = None;
        let mut finished_label: Option<String> = None;
        if let Some(job) = self.state.workflow_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    WorkflowMsg::Progress(step) => job.current_step = Some(step),
                    WorkflowMsg::Done(_) | WorkflowMsg::Error(_) | WorkflowMsg::Patch(_) => {
                        finished_label = Some(job.label.clone());
                        finished = Some(msg)
                    }
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.workflow_job = None;
            match res {
                WorkflowMsg::Progress(_) => {}
                WorkflowMsg::Done(s) => {
                    if matches!(
                        finished_label.as_deref(),
                        Some("stage resolved files and continue") | Some("abort merge")
                    ) {
                        self.state.conflict_followup = None;
                        self.state.conflicts.clear();
                    } else if !matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_followup = None;
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_log = s.clone();
                    } else {
                        self.state.modal = Modal::None;
                    }
                    self.state.set_status(first_status_line(&s), false);
                }
                WorkflowMsg::Patch(patch) => {
                    self.state.pending_llm_patch = Some(patch);
                    self.state.conflict_log =
                        "LLM patch ready. Review preview and press p to apply.".to_string();
                    self.state.modal = Modal::Conflict;
                    self.state.set_status("LLM patch ready", false);
                }
                WorkflowMsg::Error(e) => {
                    if let Ok(conflicts) = crate::git::conflicted_files() {
                        if !conflicts.is_empty() {
                            self.state.conflicts = conflicts;
                            self.state.conflict_idx = 0;
                            self.state.conflict_log = e.clone();
                            self.state.modal = Modal::Conflict;
                            self.state.set_status("merge conflicts detected", true);
                            self.start_refresh(true);
                            return Ok(());
                        }
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_log = e.clone();
                    }
                    if !matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_followup = None;
                    }
                    self.state.set_status(first_status_line(&e), true);
                }
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_generation(&mut self) {
        let mut drained: Vec<GenMsg> = Vec::new();
        if let Some(g) = self.state.generation.as_ref() {
            while let Ok(msg) = g.rx.try_recv() {
                drained.push(msg);
            }
        }
        for msg in drained {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(o) => {
                    if let Some(g) = self.state.generation.as_mut() {
                        g.output.push_str(&o);
                    }
                }
                GenMsg::Done(final_msg) => {
                    self.state.commit_message = final_msg;
                    self.state.generation = None;
                    self.state.set_status("message generated", false);
                }
                GenMsg::Error(e) => {
                    self.state.generation = None;
                    self.state.set_status(e, true);
                }
            }
        }
        if let Some(g) = self.state.generation.as_mut() {
            g.spinner = g.spinner.wrapping_add(1);
        }
    }

    fn render(&mut self) -> Result<()> {
        self.state.advance_animation();

        // Compute viewport height before the draw closure so we can update state.
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects_pre = ui::split_layout_with_environments(area, self.state.flow_available());
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout_with_environments(area, state.flow_available());
            let focused_pane = state.focus;

            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
            panel::environments::render(state, rects.environments, frame);
            panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
            panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
            panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
            panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

            draw_footer(frame, rects.footer, state);

            match state.modal {
                Modal::None => {}
                Modal::Commit => panel::commit::render(state, area, frame),
                Modal::Push => panel::push::render(state, area, frame),
                Modal::Help => panel::help::render(state, area, frame),
                Modal::Flow => panel::flow::render(state, area, frame),
                Modal::Conflict => panel::conflict::render(state, area, frame),
            }
        })?;
        Ok(())
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> Result<()> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return Ok(());
        }

        match self.state.modal {
            Modal::Help => {
                panel::help::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Commit => {
                panel::commit::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Push => {
                panel::push::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Flow => {
                panel::flow::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Conflict => {
                panel::conflict::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::None => {}
        }

        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
                return Ok(());
            }
            KeyCode::Char('F') => {
                self.start_refresh(false);
                if self.state.flow_available() {
                    self.state.modal = Modal::Flow;
                }
                return Ok(());
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.state.should_quit = true;
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
                self.start_diff_job(false);
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
                self.start_diff_job(false);
                return Ok(());
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
                self.start_diff_job(false);
                return Ok(());
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
                self.start_diff_job(false);
                return Ok(());
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
                return Ok(());
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
                self.start_diff_job(false);
                return Ok(());
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
                self.start_diff_job(false);
                return Ok(());
            }
            KeyCode::Char('c') => {
                self.state.open_commit_modal();
                return Ok(());
            }
            KeyCode::Char('p') => {
                self.start_refresh(false);
                self.state.modal = Modal::Push;
                return Ok(());
            }
            KeyCode::Char('P') => {
                if self.state.unpushed_shas.is_empty() {
                    self.state.set_status("nothing to push", false);
                    return Ok(());
                }
                spawn_push(&mut self.state);
                return Ok(());
            }
            _ => {}
        }

        let focus_before = self.state.focus;

        match focus_before {
            Pane::Status => {}
            Pane::Files => panel::files::handle_key(&mut self.state, k)?,
            Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
            Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
            Pane::Main => panel::main::handle_key(&mut self.state, k)?,
        }

        if self.state.pending_action.is_none()
            && (matches!(focus_before, Pane::Files | Pane::Branches | Pane::Commits)
                || matches!(
                    self.state.focus,
                    Pane::Files | Pane::Branches | Pane::Commits
                ))
        {
            self.start_diff_job(false);
        }
        Ok(())
    }

    fn handle_mouse(&mut self, m: MouseEvent) -> Result<()> {
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects = ui::split_layout_with_environments(area, self.state.flow_available());
        let in_main = m.column >= rects.main.x
            && m.column < rects.main.x + rects.main.width
            && m.row >= rects.main.y
            && m.row < rects.main.y + rects.main.height;
        if !in_main {
            return Ok(());
        }
        let max_offset = self
            .state
            .diff_line_count
            .saturating_sub(self.state.diff_viewport_height);
        match m.kind {
            MouseEventKind::ScrollDown => {
                self.state.diff_offset = self.state.diff_offset.saturating_add(3).min(max_offset);
            }
            MouseEventKind::ScrollUp => {
                self.state.diff_offset = self.state.diff_offset.saturating_sub(3);
            }
            _ => {}
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}
