//! Starting the app's background jobs and taking in what they send back.

use std::thread::JoinHandle;

use crate::state::{BackgroundJob, GenMsg, Modal, ReviewAssistJob};

mod conflict;
mod drain;
mod review;
mod start;

/// Take a finished single-shot job out of `slot`, with the last message it sent.
/// `None` while it is still running, in which case its spinner advances.
fn take_finished<J: BackgroundJob>(slot: &mut Option<J>) -> Option<(J, J::Msg)> {
    let job = slot.as_mut()?;
    let mut finished = None;
    while let Ok(msg) = job.rx().try_recv() {
        finished = Some(msg);
    }
    match finished {
        Some(msg) => Some((slot.take()?, msg)),
        None => {
            tick_spinner(slot);
            None
        }
    }
}

/// Everything a streaming job has sent since the last check. The job stays put:
/// it reports many times before it is done.
fn drain_messages<J: BackgroundJob>(slot: &Option<J>) -> Vec<J::Msg> {
    let Some(job) = slot.as_ref() else {
        return Vec::new();
    };
    let mut drained = Vec::new();
    while let Ok(msg) = job.rx().try_recv() {
        drained.push(msg);
    }
    drained
}

fn tick_spinner<J: BackgroundJob>(slot: &mut Option<J>) {
    if let Some(job) = slot.as_mut() {
        let spinner = job.spinner_mut();
        *spinner = spinner.wrapping_add(1);
    }
}

/// Stream an LLM answer for a review node into the pane. Review explanations and
/// PR text differ only in what they are called when they finish. Returns the
/// status to show, if the stream reached an end.
fn drain_review_stream(
    slot: &mut Option<ReviewAssistJob>,
    assists: &mut std::collections::HashMap<String, String>,
    ready: &'static str,
) -> Option<(String, bool)> {
    let mut status = None;
    let mut handle = None;
    for msg in drain_messages(slot) {
        match msg {
            GenMsg::Thinking(_) => {}
            GenMsg::Output(output) => {
                if let Some(job) = slot.as_mut() {
                    job.output.push_str(&output);
                    assists.insert(job.node_id.clone(), job.output.clone());
                }
            }
            GenMsg::Reset => {
                if let Some(job) = slot.as_mut() {
                    job.output.clear();
                    assists.insert(job.node_id.clone(), String::new());
                }
            }
            GenMsg::Done {
                text: final_msg,
                stats,
            } => {
                let truncated = stats.truncated;
                let final_msg = mark_if_truncated(final_msg, truncated);
                if let Some(mut job) = slot.take() {
                    handle = job.handle.take();
                    assists.insert(job.node_id, final_msg);
                }
                status = Some((
                    if truncated {
                        format!("{ready} \u{2014} cut off at the token budget")
                    } else {
                        ready.to_string()
                    },
                    truncated,
                ));
            }
            GenMsg::Error(error) => {
                if let Some(mut job) = slot.take() {
                    handle = job.handle.take();
                    assists.insert(job.node_id, format!("llm error: {error}"));
                }
                status = Some((error, true));
            }
        }
    }
    join_worker(handle);
    tick_spinner(slot);
    status
}

/// Say in the text itself that an answer was cut off.
///
/// The status line that says so expires; the answer stays on screen and gets
/// copied into a PR description, so the one place the notice cannot be missed
/// is the answer.
pub(super) fn mark_if_truncated(text: String, truncated: bool) -> String {
    if !truncated || text.trim().is_empty() {
        return text;
    }
    format!("{}\n\n{}", text.trim_end(), crate::llm::TRUNCATED_NOTE)
}

fn join_worker(handle: Option<JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
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

fn open_conflict_modal_if_needed(state: &mut crate::state::AppState, log: String) -> bool {
    let conflicts = crate::git::conflicted_files().unwrap_or_default();
    if conflicts.is_empty() {
        return false;
    }
    state.set_conflicts(conflicts);
    state.conflict_log = log;
    state.modal = Modal::Conflict;
    state.set_status("conflicts detected", true);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A status line expires; an answer stays on screen and gets pasted into a
    /// pull request. An answer the server stopped at the budget reads exactly
    /// like one that finished, so the notice belongs in the answer itself.
    #[test]
    fn a_cut_off_answer_says_so_in_its_own_text() {
        let marked = mark_if_truncated("## Summary\n- did a thing".to_string(), true);

        assert!(marked.starts_with("## Summary\n- did a thing"));
        assert!(marked.contains(crate::llm::TRUNCATED_NOTE), "{marked}");
    }

    #[test]
    fn a_complete_answer_is_left_exactly_as_it_came() {
        let text = "## Summary\n- did a thing".to_string();

        assert_eq!(mark_if_truncated(text.clone(), false), text);
    }

    #[test]
    fn there_is_nothing_to_mark_on_an_empty_answer() {
        assert_eq!(mark_if_truncated(String::new(), true), "");
    }
}
