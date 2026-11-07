//! The Claude Code chat beside a pull request's conversations.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::panel::github::Mode;
use crate::panel::github::threads::chat::{ThreadsChat, brief};
use crate::session::{SessionKind, SessionSpec};
use crate::state::AppState;

use super::App;

impl App {
    pub(super) fn open_threads_chat(&mut self) {
        match start(&mut self.state) {
            Ok(location) => self.state.set_status(
                format!(
                    "Claude is reading the code on {location} \u{2014} Ctrl-] back to the conversations"
                ),
                false,
            ),
            Err(e) => self
                .state
                .set_status(format!("chat failed to start: {e:#}"), true),
        }
    }
}

/// Start Claude in the checkout, handed the selected conversation, with the
/// keyboard.
fn start(state: &mut AppState) -> Result<String> {
    let Mode::Threads(view) = &state.github.mode else {
        anyhow::bail!("no pull request's conversations open");
    };
    let thread = view.selected().context("no conversation selected")?;
    let (number, thread_id, location) = (view.number, thread.id.clone(), thread.location());
    let prompt = crate::llm::build_threads_chat_prompt(
        number,
        &view.title,
        &brief(thread),
        &crate::settings::load(),
    );
    let cwd = PathBuf::from(state.repo_root.as_deref().context("no repository open")?);
    let profile = crate::agents::claude_profile();
    if profile.sandboxed() {
        super::actions::prepare_sandbox(&cwd)?;
    }
    let spec = SessionSpec {
        label: format!("conversations \u{b7} #{number}"),
        cwd,
        sandboxed: profile.sandboxed(),
        kind: SessionKind::Claude,
        prompt: Some(prompt),
    };
    let id = state
        .sessions
        .start_profile(spec, &profile, crate::session::default_size())?;
    state.github.threads_chat = Some(ThreadsChat {
        number,
        id,
        thread: thread_id,
        location: location.clone(),
        focused: true,
    });
    Ok(location)
}
