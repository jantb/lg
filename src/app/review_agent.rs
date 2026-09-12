//! A Claude Code session reviewing the whole branch, its findings read back
//! into the review tree.
//!
//! The local model sees what lg hands it; a session has the checkout and can
//! read callers and tests for itself. It reports the same way the hooks do —
//! by writing a file lg chose — because the session's screen is a terminal,
//! and reading a review off a terminal is guessing.

use std::path::PathBuf;

use anyhow::{Context, Result};
use ratatui::backend::Backend;

use crate::git::REVIEW_AGENT_NODE_ID;
use crate::session::{SessionKind, SessionSpec};
use crate::state::{AppState, ReviewAgentJob};

use super::{App, HeadlessApp};

impl App {
    /// Start a Claude Code session over the checkout with the review as its
    /// opening prompt, and show it. One review session per checkout: a second
    /// press while it runs goes back to the running one.
    pub(super) fn start_review_agent(&mut self) {
        if let Some(id) = self.state.review_agent_job.as_ref().map(|job| job.session)
            && self.state.sessions.get(id).is_some()
        {
            self.state.show_session(id);
            self.set_session_capture(true);
            self.state
                .set_status("agent review already running \u{2014} showing it", false);
            return;
        }
        match start_review_agent(&mut self.state) {
            Ok(id) => {
                self.state.show_session(id);
                self.set_session_capture(true);
                self.state
                    .set_status("agent review started \u{2014} Ctrl-] returns to lg", false);
            }
            Err(e) => self
                .state
                .set_status(format!("agent review failed to start: {e:#}"), true),
        }
    }

    pub(in crate::app) fn drain_review_agent(&mut self) {
        if let Some((text, is_error)) = poll_review_agent(&mut self.state) {
            self.state.set_status(text, is_error);
        }
    }
}

impl<B: Backend> HeadlessApp<B> {
    /// What the real loop does for the agent review each frame. This is the
    /// seam tests drive.
    pub fn drain_review_agent(&mut self) {
        if let Some((text, is_error)) = poll_review_agent(&mut self.state) {
            self.state.set_status(text, is_error);
        }
    }
}

fn start_review_agent(state: &mut AppState) -> Result<crate::session::SessionId> {
    let review = state
        .review
        .as_ref()
        .context("build the review first (R)")?;
    let cwd = PathBuf::from(
        state
            .repo_root
            .as_deref()
            .context("no repository open to review")?,
    );
    let findings = crate::hooks::review_findings_path(&cwd)?;
    let branches = crate::preferences::load().config.branches;
    let base_ref = format!("{}/{}", branches.remote, branches.base);
    let prompt = crate::llm::build_review_agent_prompt(
        &review.report,
        &base_ref,
        &findings,
        &crate::settings::load(),
    );
    let profile = claude_profile();
    if profile.sandboxed() {
        super::actions::prepare_sandbox(&cwd)?;
    }
    let spec = SessionSpec {
        label: format!(
            "review · {}",
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
    let label = state
        .sessions
        .get(id)
        .map(|session| session.label.clone())
        .unwrap_or_default();
    state.review_assists.insert(
        REVIEW_AGENT_NODE_ID.to_string(),
        format!(
            "Claude Code is reviewing the branch in the session \"{label}\".\n\
             Its findings appear here as soon as it writes them; Ctrl-n / Ctrl-p shows the session."
        ),
    );
    state.review_agent_job = Some(ReviewAgentJob {
        session: id,
        findings,
        seen: String::new(),
    });
    Ok(id)
}

/// The claude profile from Settings, the default one when several are
/// configured, or a plain `claude` when none is: the review needs claude
/// specifically, because it is the adapter whose findings lg knows how to ask
/// for.
fn claude_profile() -> crate::preferences::Agent {
    let agents = crate::preferences::load().config.agents;
    agents
        .iter()
        .find(|agent| agent.adapter == "claude" && agent.default)
        .or_else(|| agents.iter().find(|agent| agent.adapter == "claude"))
        .cloned()
        .unwrap_or_else(|| crate::preferences::Agent {
            name: "Claude".into(),
            adapter: "claude".into(),
            executable: "claude".into(),
            confinement: crate::preferences::default_confinement("claude").into(),
            ..Default::default()
        })
}

/// Read the findings file if it changed, and settle the job once its session
/// has gone. Returns what to say about it, if anything.
pub(crate) fn poll_review_agent(state: &mut AppState) -> Option<(String, bool)> {
    let job = state.review_agent_job.as_mut()?;
    let mut notice = None;
    if let Ok(text) = std::fs::read_to_string(&job.findings)
        && !text.trim().is_empty()
        && text != job.seen
    {
        let first_time = job.seen.is_empty();
        job.seen = text.clone();
        state
            .review_assists
            .insert(REVIEW_AGENT_NODE_ID.to_string(), text);
        if first_time {
            notice = Some(("agent review findings ready (y copy)".to_string(), false));
        }
    }
    let session_gone = state.sessions.get(job.session).is_none();
    if !session_gone {
        return notice;
    }
    let job = state.review_agent_job.take()?;
    if job.seen.is_empty() {
        state.review_assists.insert(
            REVIEW_AGENT_NODE_ID.to_string(),
            format!(
                "The review session ended without writing its findings to\n{}",
                job.findings.display()
            ),
        );
        return Some(("agent review ended without findings".to_string(), true));
    }
    notice.or(Some(("agent review finished".to_string(), false)))
}
