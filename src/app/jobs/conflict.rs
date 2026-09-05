//! Taking in what the local model made of the conflict, and calling claude in
//! when it did not get there.

use crate::state::ConflictResolveMsg;

use super::super::App;
use super::{drain_messages, join_worker, tick_spinner};

impl App {
    pub(in crate::app) fn drain_conflict_resolve_job(&mut self) {
        let mut handle = None;
        for msg in drain_messages(&self.state.conflict_resolve_job) {
            match msg {
                ConflictResolveMsg::Started { path, index, total } => {
                    if let Some(job) = self.state.conflict_resolve_job.as_mut() {
                        job.active_path = Some(path.clone());
                    }
                    self.state
                        .set_status(format!("resolving {index}/{total}: {path}"), false);
                }
                ConflictResolveMsg::Resolved { path, hunks } => {
                    self.advance_conflict_resolve(&path);
                    self.state.conflict_resolved.insert(path.clone());
                    self.state.set_status(
                        format!("local model settled {hunks} conflict(s) in {path}"),
                        false,
                    );
                }
                ConflictResolveMsg::Declined { path, reason } => {
                    self.advance_conflict_resolve(&path);
                    let agent = self.state.preferred_agent.label();
                    self.state
                        .set_status(format!("left {path} to {agent}: {reason}"), false);
                }
                ConflictResolveMsg::Finished { resolved, declined } => {
                    if let Some(job) = self.state.conflict_resolve_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.conflict_resolve_job = None;
                    self.finish_conflict_resolve(resolved, declined);
                }
            }
        }
        join_worker(handle);
        tick_spinner(&mut self.state.conflict_resolve_job);
    }

    fn advance_conflict_resolve(&mut self, path: &str) {
        if let Some(job) = self.state.conflict_resolve_job.as_mut() {
            job.completed = job.completed.saturating_add(1);
            if job.active_path.as_deref() == Some(path) {
                job.active_path = None;
            }
        }
    }

    /// Say what the pass came to, and hand what is left to the chosen agent.
    ///
    /// Falling back without being asked again is the whole arrangement: the
    /// local attempt is worth making precisely because nothing is lost when it
    /// does not work out, and stopping here to ask would spend the time it
    /// saved. The files it did settle stay unstaged and on the list, so they
    /// can be read before `v` commits them.
    fn finish_conflict_resolve(&mut self, resolved: Vec<String>, declined: Vec<String>) {
        let mut log = String::new();
        if !resolved.is_empty() {
            log.push_str("Resolved by the local model (review before validating):\n");
            for path in &resolved {
                log.push_str(&format!("- {path}\n"));
            }
        }
        if !declined.is_empty() {
            if !log.is_empty() {
                log.push('\n');
            }
            log.push_str(&format!(
                "Left for {}:\n",
                self.state.preferred_agent.label()
            ));
            for path in &declined {
                log.push_str(&format!("- {path}\n"));
            }
        }
        self.state.conflict_log = log;

        if declined.is_empty() {
            let count = resolved.len();
            self.state.set_status(
                format!(
                    "local model resolved {count} file(s) \u{2014} read them with o, then v to continue"
                ),
                false,
            );
            return;
        }

        self.state.set_status(
            format!(
                "{} file(s) were past the local model \u{2014} handing them to {}",
                declined.len(),
                self.state.preferred_agent.label()
            ),
            false,
        );
        super::super::start_conflict_session(&mut self.state, true);
    }
}
