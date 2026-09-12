//! Every running session, and starting, focusing, closing and pumping them.

use super::*;

/// A session that has stopped and been dropped from the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedSession {
    pub id: SessionId,
    /// The checkout it was running in.
    pub label: String,
    /// How it ended, in the words the session settled on: "exited", "killed by
    /// 9", "stopped".
    pub notice: String,
}

impl EndedSession {
    /// What is worth keeping about `session` once it is gone.
    pub(super) fn of(session: &Session) -> Self {
        Self {
            id: session.id,
            label: session.label.clone(),
            notice: match &session.status {
                SessionStatus::Ended(notice) => notice.clone(),
                SessionStatus::Running => "stopped".to_string(),
            },
        }
    }
}

/// Every live session, and which one is being shown.
pub struct Sessions {
    pub(super) items: Vec<Session>,
    pub(super) focused: Option<SessionId>,
    pub(super) next_id: u64,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            focused: None,
            next_id: 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Session> {
        self.items.iter()
    }

    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.items.iter().find(|session| session.id == id)
    }

    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.items.iter_mut().find(|session| session.id == id)
    }

    pub fn focused(&self) -> Option<SessionId> {
        self.focused
    }

    pub fn focused_session(&self) -> Option<&Session> {
        self.focused.and_then(|id| self.get(id))
    }

    /// Show this session, and clear the "look at me" mark it may carry.
    pub fn focus(&mut self, id: SessionId) {
        if let Some(session) = self.get_mut(id) {
            session.attention = false;
            self.focused = Some(id);
        }
    }

    /// Every session running in `dir`, in the order they were started. A
    /// checkout can have one of each kind, so this is what the tree lists under
    /// it.
    pub fn for_dir(&self, dir: &Path) -> impl Iterator<Item = &Session> {
        self.items
            .iter()
            .filter(move |session| same_dir(&session.cwd, dir))
    }

    /// The `kind` session running in `dir`, if there is one. One session of
    /// each kind per checkout is the whole point: asking for a terminal in a
    /// worktree twice finds the first one again.
    pub fn for_dir_kind(&self, dir: &Path, kind: SessionKind) -> Option<SessionId> {
        self.for_dir(dir)
            .find(|session| session.kind == kind)
            .map(|session| session.id)
    }

    /// Whether anything at all is running in `dir`. Removing a checkout out
    /// from under any of it would pull the rug on a live process.
    pub fn any_in_dir(&self, dir: &Path) -> bool {
        self.for_dir(dir).next().is_some()
    }

    /// Number of sessions that have drawn something since they were last
    /// looked at. This is what the caret on a tree row means: unread output,
    /// which for a session mid-turn is nearly always.
    pub fn attention_count(&self) -> usize {
        self.items
            .iter()
            .filter(|session| session.attention)
            .count()
    }

    /// How many sessions are in each state worth saying out loud: blocked on a
    /// question, and busy. Idle sessions are the rest, and need no number.
    ///
    /// Unread output is deliberately not folded in here. A session that has
    /// printed a line is not waiting for anybody, and counting it as though it
    /// were makes the badge cry wolf for as long as anything is running.
    pub fn activity_counts(&self) -> (usize, usize) {
        self.items
            .iter()
            .fold((0, 0), |(needs_input, working), session| {
                match session.activity() {
                    SessionActivity::NeedsInput => (needs_input + 1, working),
                    SessionActivity::Working | SessionActivity::Running => {
                        (needs_input, working + 1)
                    }
                    SessionActivity::Idle => (needs_input, working),
                }
            })
    }

    /// Start the session `spec` asks for, or hand back the one of that kind
    /// already running there.
    pub fn start(&mut self, spec: SessionSpec, size: (u16, u16)) -> Result<SessionId> {
        if let Some(existing) = self.for_dir_kind(&spec.cwd, spec.kind) {
            self.focus(existing);
            return Ok(existing);
        }
        match spec.kind {
            SessionKind::Claude => self.start_claude(spec, size),
            SessionKind::Codex => {
                let spawn = codex_spawn(&spec.cwd, spec.sandboxed, spec.prompt.as_deref());
                self.start_with(spec, &spawn, size)
            }
            SessionKind::Pi => {
                let spawn = pi_spawn(&spec.cwd, spec.sandboxed, spec.prompt.as_deref());
                self.start_with(spec, &spawn, size)
            }
            SessionKind::Terminal => {
                let spawn = shell_spawn(&spec.cwd);
                self.start_with(spec, &spawn, size)
            }
        }
    }

    pub fn start_profile(
        &mut self,
        spec: SessionSpec,
        profile: &crate::preferences::Agent,
        size: (u16, u16),
    ) -> Result<SessionId> {
        let hooks = if profile.adapter == "claude" {
            crate::hooks::install(&spec.cwd).ok()
        } else {
            None
        };
        let spawn = crate::agents::spawn(
            profile,
            &spec.cwd,
            hooks.as_ref().map(|h| h.settings.as_path()),
            spec.prompt.as_deref(),
        )?;
        let id = self.start_with(spec, &spawn, size)?;
        if let Some(session) = self.get_mut(id) {
            session.events = hooks.map(|h| h.events);
        }
        Ok(id)
    }

    /// Start claude, wired up to report what it is doing. A checkout with
    /// nowhere to keep a hook file still gets a session; it just has to do
    /// without claude saying what it is up to.
    pub(super) fn start_claude(
        &mut self,
        spec: SessionSpec,
        size: (u16, u16),
    ) -> Result<SessionId> {
        let hooks = crate::hooks::install(&spec.cwd).ok();
        let settings = hooks.as_ref().map(|channel| channel.settings.as_path());
        let spawn = claude_spawn(&spec.cwd, spec.sandboxed, settings, spec.prompt.as_deref());
        let id = self.start_with(spec, &spawn, size)?;
        if let Some(session) = self.get_mut(id) {
            session.events = hooks.map(|channel| channel.events);
        }
        Ok(id)
    }

    /// Start a session running something other than claude. This is the seam a
    /// different session backend plugs into, and what tests drive.
    pub fn start_with(
        &mut self,
        spec: SessionSpec,
        spawn: &Spawn,
        size: (u16, u16),
    ) -> Result<SessionId> {
        if let Some(existing) = self
            .items
            .iter()
            .find(|s| s.cwd == spec.cwd && s.kind == spec.kind && s.label == spec.label)
            .map(|s| s.id)
        {
            self.focus(existing);
            return Ok(existing);
        }

        let size = (size.0.max(1), size.1.max(1));
        let process = PtyProcess::start(spawn, size)?;
        let id = SessionId(self.next_id);
        self.next_id += 1;
        let launch = Some((spec.clone(), spawn.clone()));
        self.items.push(Session {
            id,
            label: spec.label,
            cwd: spec.cwd,
            sandboxed: spec.sandboxed,
            kind: spec.kind,
            status: SessionStatus::Running,
            launch,
            attention: false,
            activity: SessionActivity::Idle,
            asking: false,
            events: None,
            last_output: None,
            parser: vt100::Parser::new(size.0, size.1, SCROLLBACK),
            process: None,
        });
        // Attaching after the push keeps the struct literal readable.
        if let Some(session) = self.items.last_mut() {
            session.process = Some(process);
        }
        self.focused = Some(id);
        Ok(id)
    }

    pub fn rename(&mut self, id: SessionId, label: &str) -> Result<()> {
        if label.trim().is_empty() {
            anyhow::bail!("session name cannot be empty");
        }
        let session = self
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("session ended"))?;
        session.label = label.trim().into();
        if let Some((spec, _)) = &mut session.launch {
            spec.label = session.label.clone();
        }
        Ok(())
    }
    pub fn restart(&mut self, id: SessionId) -> Result<SessionId> {
        let (spec, spawn) = self
            .get(id)
            .and_then(|s| s.launch.clone())
            .ok_or_else(|| anyhow::anyhow!("no saved launch command for this session"))?;
        self.close(id);
        self.start_with(spec, &spawn, default_size())
    }

    /// Stop a session and forget it. The next session in the list takes focus,
    /// so closing one does not leave the pane pointing at nothing.
    pub fn close(&mut self, id: SessionId) {
        let Some(idx) = self.items.iter().position(|session| session.id == id) else {
            return;
        };
        self.items.remove(idx);
        if self.focused == Some(id) {
            self.focused = self
                .items
                .get(idx)
                .or_else(|| self.items.last())
                .map(|session| session.id);
        }
    }

    /// Read every session's output, and let go of the ones whose program has
    /// ended. A stopped session has nothing left to draw and nothing to type
    /// into, so it goes as soon as it stops rather than staying on as a row
    /// waiting to be dismissed by hand.
    ///
    /// Returns what was dropped, so the caller can say what happened to it.
    pub fn pump(&mut self) -> Vec<EndedSession> {
        let focused = self.focused;
        for session in &mut self.items {
            session.pump(focused == Some(session.id));
        }
        let ended: Vec<EndedSession> = self
            .items
            .iter()
            .filter(|session| !session.is_running())
            .map(EndedSession::of)
            .collect();
        for session in &ended {
            self.close(session.id);
        }
        ended
    }

    /// The session after (or before) the one being shown, wrapping round. Used
    /// for cycling between sessions without going through the tree.
    pub fn neighbour(&self, forward: bool) -> Option<SessionId> {
        if self.items.is_empty() {
            return None;
        }
        let current = self
            .focused
            .and_then(|id| self.items.iter().position(|session| session.id == id))
            .unwrap_or(0);
        let len = self.items.len();
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        Some(self.items[next].id)
    }

    /// Kill everything, on the way out.
    pub fn close_all(&mut self) {
        self.items.clear();
        self.focused = None;
    }
}
