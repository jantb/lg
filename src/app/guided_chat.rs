//! The Claude Code chat a guided review opens beside the hunk.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::panel::guided::{ChatNews, Mode};
use crate::session::{SessionId, SessionKind, SessionSpec};
use crate::state::AppState;

use super::App;

impl App {
    pub(super) fn open_guided_chat(&mut self) {
        match start(&mut self.state) {
            Ok(path) => self.state.set_status(
                format!("chat about {path} \u{2014} Ctrl-] back to the review"),
                false,
            ),
            Err(e) => self
                .state
                .set_status(format!("review chat failed to start: {e:#}"), true),
        }
    }
}

/// Returns the file the chat starts on.
fn start(state: &mut AppState) -> Result<String> {
    let guided = state.guided.as_ref().context("no guided review open")?;
    let path = guided
        .step()
        .map(|step| step.path.clone())
        .context("no file on screen")?;
    let cwd = PathBuf::from(state.repo_root.as_deref().context("no repository open")?);
    let read = guided.has_read(&path);
    let brief = if read {
        guided.chat_brief(&path)
    } else {
        format!("lg is still reading {path}; what is open there follows once it is read.")
    };
    let prompt = crate::llm::build_guided_chat_prompt(
        &guided.source.title(),
        &brief,
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
    let id: SessionId =
        state
            .sessions
            .start_profile(spec, &profile, crate::session::default_size())?;
    if let Some(guided) = state.guided.as_mut() {
        guided.chat = Some(id);
        guided.chat_file = Some(path.clone());
        guided.mode = Mode::Chat;
        if !read {
            guided.owe_chat(path.clone(), ChatNews::FileRead);
        }
    }
    Ok(path)
}
