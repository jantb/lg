use anyhow::{Context, Result};
use chrono::Utc;
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
use std::{io::Stdout, time::Duration};

use crate::{
    config::{COMMIT_LIST_LIMIT, DEFAULT_PUSH_REMOTE, STATUS_MSG_LIFETIME_SECS, TICK_MS},
    panel,
    state::{AppState, DiffSource, GenMsg, Modal, Pane, PendingAction, PushJob, PushMsg, TreeKind},
    ui,
};

pub struct App {
    pub state: AppState,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
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

fn spawn_push(state: &mut AppState) {
    if state.push_job.is_some() {
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

fn footer_spec(pane: Pane) -> (u8, &'static str, &'static [(&'static str, &'static str)]) {
    match pane {
        Pane::Status => (1, "Status", &[("?", "help"), ("q", "quit")]),
        Pane::Files => (
            2,
            "Files",
            &[
                ("space", "stage"),
                ("u", "unstage"),
                ("A/U", "all"),
                ("c", "commit"),
                ("p/P", "push"),
                ("?", "help"),
            ],
        ),
        Pane::Branches => (3, "Branches", &[("Enter", "checkout"), ("?", "help")]),
        Pane::Commits => (
            4,
            "Commits",
            &[("j/k", "navigate"), ("Enter", "focus diff"), ("?", "help")],
        ),
        Pane::Main => (
            0,
            "Diff",
            &[("j/k", "scroll"), ("g/G", "top/bot"), ("?", "help")],
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
                spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
                spans.push(Span::raw(" "));
                spans.push(Span::raw(*label));
                if i + 1 < pairs.len() {
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
        let rects_pre = ui::split_layout(area);
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout(area);
            let focused_pane = state.focus;

            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
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
            Modal::None => {}
        }
        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
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
                    let branch = self.state.branch.clone().unwrap_or_default();
                    match crate::git::push(DEFAULT_PUSH_REMOTE, &branch) {
                        Ok(out) => {
                            let line = out
                                .lines()
                                .rfind(|l| !l.trim().is_empty())
                                .unwrap_or("pushed")
                                .to_owned();
                            self.state.set_status(line, false);
                        }
                        Err(e) => self.state.set_status(e.to_string(), true),
                    }
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
        };
        app.refresh()?;
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

            let poll_ms = if self.state.generation.is_some() || self.state.push_job.is_some() {
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
                        match crate::git::commit(&msg) {
                            Ok(out) => {
                                let line = out.lines().next().unwrap_or("committed").to_owned();
                                self.state.set_status(line, false);
                                self.state.modal = Modal::None;
                                self.state.commit_message.clear();
                                self.refresh()?;
                            }
                            Err(e) => {
                                self.state.set_status(e.to_string(), true);
                            }
                        }
                    }
                    PendingAction::Push => {
                        spawn_push(&mut self.state);
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

    fn recompute_diff_source(&mut self) -> &mut Self {
        let new_source = match self.state.focus {
            Pane::Files => {
                let rows = self.state.tree_rows();
                match rows.get(self.state.files_idx) {
                    Some(row) => match &row.kind {
                        TreeKind::AllChanges => DiffSource::All,
                        TreeKind::Folder { .. } => DiffSource::Folder(row.path.clone()),
                        TreeKind::File { entry_idx } => self
                            .state
                            .files
                            .get(*entry_idx)
                            .map(|f| DiffSource::File(f.path.clone()))
                            .unwrap_or(DiffSource::None),
                    },
                    None => DiffSource::None,
                }
            }
            Pane::Commits => self
                .state
                .commits
                .get(self.state.commits_idx)
                .map(|c| DiffSource::Commit(c.sha.clone()))
                .unwrap_or(DiffSource::None),
            Pane::Branches => self
                .state
                .branches
                .get(self.state.branches_idx)
                .map(|b| DiffSource::Branch(b.name.clone()))
                .unwrap_or(DiffSource::None),
            _ => DiffSource::None,
        };
        if new_source != self.state.diff_source {
            self.state.diff_source = new_source.clone();
            self.state.diff_offset = 0;
            self.state.diff_text = match &new_source {
                DiffSource::None => String::new(),
                DiffSource::All => {
                    crate::git::all_diffs().unwrap_or_else(|e| format!("error: {e}"))
                }
                DiffSource::File(p) => {
                    crate::git::file_diff(p).unwrap_or_else(|e| format!("error: {e}"))
                }
                DiffSource::Folder(p) => {
                    crate::git::folder_diff(p).unwrap_or_else(|e| format!("error: {e}"))
                }
                DiffSource::Commit(sha) => {
                    crate::git::show_commit(sha).unwrap_or_else(|e| format!("error: {e}"))
                }
                DiffSource::Branch(_) => String::new(),
            };
            self.state.diff_line_count =
                self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
        }
        self
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
            self.refresh()?;
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

    fn refresh(&mut self) -> Result<()> {
        if let Ok(files) = crate::git::status_entries() {
            self.state.files = files;
        }
        if let Ok(branches) = crate::git::list_branches() {
            self.state.branches = branches;
        }
        match crate::git::list_commits(COMMIT_LIST_LIMIT) {
            Ok(commits) => self.state.commits = commits,
            Err(e) => self.state.set_status(format!("git log failed: {e}"), true),
        }
        match crate::git::unpushed_shas() {
            Ok(shas) => self.state.unpushed_shas = shas,
            Err(e) => self
                .state
                .set_status(format!("unpushed check failed: {e}"), true),
        }
        self.state.branch = crate::git::head_branch().ok();
        self.state.remote_url = crate::git::remote_url(DEFAULT_PUSH_REMOTE).ok();
        self.state.ahead_behind = crate::git::counts_ahead_behind().ok();
        self.state.clamp();
        self.recompute_diff_source();
        Ok(())
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
        let rects_pre = ui::split_layout(area);
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout(area);
            let focused_pane = state.focus;

            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
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
            Modal::None => {}
        }

        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
                return Ok(());
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.state.should_quit = true;
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
                self.recompute_diff_source();
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
                self.recompute_diff_source();
                return Ok(());
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
                self.recompute_diff_source();
                return Ok(());
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
                self.recompute_diff_source();
                return Ok(());
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
                return Ok(());
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
                self.recompute_diff_source();
                return Ok(());
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
                self.recompute_diff_source();
                return Ok(());
            }
            KeyCode::Char('c') => {
                self.state.open_commit_modal();
                return Ok(());
            }
            KeyCode::Char('p') => {
                self.state.branch = crate::git::head_branch().ok();
                self.state.remote_url = crate::git::remote_url(DEFAULT_PUSH_REMOTE).ok();
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

        match self.state.focus {
            Pane::Status => {}
            Pane::Files => panel::files::handle_key(&mut self.state, k)?,
            Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
            Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
            Pane::Main => panel::main::handle_key(&mut self.state, k)?,
        }

        if matches!(
            self.state.focus,
            Pane::Files | Pane::Branches | Pane::Commits
        ) {
            self.recompute_diff_source();
        }

        self.refresh()?;
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
        let rects = ui::split_layout(area);
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
