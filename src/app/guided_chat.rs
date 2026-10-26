//! A Claude Code chat opened from a guided review, to fix what the walk found
//! with the whole checkout to hand. One per walk: asking again goes back to
//! the chat already open, so a fix can be talked through over the session.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::session::{SessionId, SessionKind, SessionSpec};
use crate::state::{AppState, Modal};

use super::App;

impl App {
    /// Show the walk's chat, starting it on what is still open if it is not
    /// running yet.
    pub(super) fn open_guided_chat(&mut self) {
        let running = self
            .state
            .guided
            .as_ref()
            .and_then(|guided| guided.chat)
            .filter(|id| self.state.sessions.get(*id).is_some());
        let id = match running.map_or_else(|| start(&mut self.state), Ok) {
            Ok(id) => id,
            Err(e) => {
                self.state
                    .set_status(format!("review chat failed to start: {e:#}"), true);
                return;
            }
        };
        self.state.modal = Modal::None;
        self.state.show_session(id);
        self.set_session_capture(true);
        self.state.set_status(
            "review chat \u{2014} Ctrl-] returns to lg, V to the review",
            false,
        );
    }
}

fn start(state: &mut AppState) -> Result<SessionId> {
    let guided = state.guided.as_ref().context("no guided review open")?;
    let cwd = PathBuf::from(state.repo_root.as_deref().context("no repository open")?);
    let prompt = crate::llm::build_guided_chat_prompt(
        &guided.source.title(),
        &guided.chat_brief(),
        &crate::settings::load(),
    );
    let profile = crate::agents::claude_profile();
    if profile.sandboxed() {
        super::actions::prepare_sandbox(&cwd)?;
    }
    let spec = SessionSpec {
        label: format!(
            "review chat \u{b7} {}",
            state.branch.clone().unwrap_or_else(|| "HEAD".to_string())
        ),
        cwd,
        sandboxed: profile.sandboxed(),
        kind: SessionKind::Claude,
        prompt: Some(prompt),
    };
    let id = state
        .sessions
        .start_profile(spec, &profile, crate::session::default_size())?;
    if let Some(guided) = state.guided.as_mut() {
        guided.chat = Some(id);
    }
    Ok(id)
}
