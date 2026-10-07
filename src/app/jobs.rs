//! Starting the app's background jobs and taking in what they send back.

use std::thread::JoinHandle;

use crate::state::{
    BackgroundJob, Drained, GenMsg, Modal, ReviewAssistJob, drain_receiver, stopped_unexpectedly,
};

mod conflict;
mod drain;
mod review;
mod start;

/// Take a finished single-shot job out of `slot`, with the last message it sent.
/// `None` while it is still running, in which case its spinner advances.
///
/// A worker that ended without a word — one that panicked — finishes the job
/// too, with `Err` carrying the status to show. Left in its slot, it would
/// spin forever and every job that waits for the slot would wait with it.
fn take_finished<J: BackgroundJob>(slot: &mut Option<J>) -> Option<(J, Result<J::Msg, String>)> {
    let job = slot.as_mut()?;
    let drained = drain_receiver(job.rx());
    match drained.messages.into_iter().last() {
        Some(msg) => Some((slot.take()?, Ok(msg))),
        None if drained.disconnected => {
            let mut job = slot.take()?;
            let status = stopped(&mut job);
            Some((job, Err(status)))
        }
        None => {
            tick_spinner(slot);
            None
        }
    }
}

/// Everything a streaming job has sent since the last check. The job stays put:
/// it reports many times before it is done.
fn drain_messages<J: BackgroundJob>(slot: &Option<J>) -> Drained<J::Msg> {
    slot.as_ref().map(drain_job).unwrap_or(Drained {
        messages: Vec::new(),
        disconnected: false,
    })
}

/// The same for a job that is not held in a slot of its own.
fn drain_job<J: BackgroundJob>(job: &J) -> Drained<J::Msg> {
    drain_receiver(job.rx())
}

/// Take a streaming job whose worker is gone out of `slot`, with the status
/// that says so — when the messages just handled did not already end it.
fn reap_stopped<J: BackgroundJob>(slot: &mut Option<J>, disconnected: bool) -> Option<(J, String)> {
    if !disconnected {
        return None;
    }
    let mut job = slot.take()?;
    let status = stopped(&mut job);
    Some((job, status))
}

/// The status for a job whose worker ended without finishing it, with the
/// panic that ended it when there is one to read.
fn stopped<J: BackgroundJob>(job: &mut J) -> String {
    stopped_status(J::NAME, job.handle_mut().take())
}

fn stopped_status(name: &str, handle: Option<JoinHandle<()>>) -> String {
    match handle.and_then(panic_message) {
        Some(reason) => format!("{}: {reason}", stopped_unexpectedly(name)),
        None => stopped_unexpectedly(name),
    }
}

/// What a worker panicked with. The channel closes while the worker is still
/// unwinding, so it is given a moment to finish; one that does not is left
/// to end on its own rather than held up for.
fn panic_message(handle: JoinHandle<()>) -> Option<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    while !handle.is_finished() {
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let payload = handle.join().err()?;
    payload
        .downcast_ref::<&str>()
        .map(|reason| (*reason).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
}

fn tick_spinner<J: BackgroundJob>(slot: &mut Option<J>) {
    if let Some(job) = slot.as_mut() {
        let spinner = job.spinner_mut();
        *spinner = spinner.wrapping_add(1);
    }
}

/// Stream an LLM answer for a review node into the pane. Review explanations and
/// PR text differ only in what they are called when they finish. Returns the
/// status to show, if the stream reached an end. A finished worker goes into
/// `deferred` to be joined once it exits, rather than waited on here.
fn drain_review_stream(
    slot: &mut Option<ReviewAssistJob>,
    assists: &mut std::collections::HashMap<String, String>,
    deferred: &mut Vec<JoinHandle<()>>,
    ready: &'static str,
) -> Option<(String, bool)> {
    let mut status = None;
    let mut handle = None;
    // The answer so far is copied into the review once per batch rather than
    // once per chunk: a long answer arrives in thousands of chunks, and copying
    // all of it each time made streaming it quadratic.
    let mut grew = false;
    let drained = drain_messages(slot);
    for msg in drained.messages {
        match msg {
            GenMsg::Thinking(_) => {}
            GenMsg::Output(output) => {
                if let Some(job) = slot.as_mut() {
                    job.output.push_str(&output);
                    grew = true;
                }
            }
            GenMsg::Reset => {
                if let Some(job) = slot.as_mut() {
                    job.output.clear();
                    grew = true;
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
    if grew && let Some(job) = slot.as_ref() {
        assists.insert(job.node_id.clone(), job.output.clone());
    }
    if let Some((job, stopped)) = reap_stopped(slot, drained.disconnected) {
        assists.insert(job.node_id, format!("llm error: {stopped}"));
        status = Some((stopped, true));
    }
    deferred.extend(handle);
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

/// Wait for a worker to exit. Only for shutdown: a worker can outlive the
/// message it sent by seconds — the Claude CLI does — and waiting on the UI
/// thread freezes the screen for that long. Use
/// [`crate::state::AppState::defer_thread_join`] everywhere else.
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
    state.conflict.log = log;
    state.modal = Modal::Conflict;
    state.set_status("conflicts detected", true);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FetchJob, FetchMsg, ReviewChatJob};
    use std::sync::mpsc::Receiver;

    /// A worker that panics before it reports, the way a bug in one does.
    fn dying_worker<M: Send + 'static>() -> (Receiver<M>, JoinHandle<()>) {
        let (tx, rx) = std::sync::mpsc::channel::<M>();
        let handle = std::thread::spawn(move || {
            let _tx = tx;
            panic!("worker bug");
        });
        while !handle.is_finished() {
            std::thread::yield_now();
        }
        (rx, handle)
    }

    /// Left in its slot, a dead job spins forever, and everything that waits
    /// for the slot to be free waits with it.
    #[test]
    fn a_job_whose_worker_died_is_finished_and_says_so() {
        let (rx, handle) = dying_worker::<FetchMsg>();
        let mut slot = Some(FetchJob {
            rx,
            handle: Some(handle),
            spinner: 0,
        });

        let (_, outcome) = take_finished(&mut slot).expect("a dead job is finished");

        assert!(slot.is_none(), "the slot is free for the next job");
        let status = outcome.expect_err("there was no message to finish with");
        assert!(status.contains("fetch stopped unexpectedly"), "{status}");
        assert!(status.contains("worker bug"), "{status}");
    }

    #[test]
    fn a_job_that_reported_before_its_worker_ended_keeps_its_message() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(FetchMsg::Done("fetched".into())).unwrap();
        drop(tx);
        let mut slot = Some(FetchJob {
            rx,
            handle: None,
            spinner: 0,
        });

        let (_, outcome) = take_finished(&mut slot).expect("finished");

        assert!(matches!(outcome, Ok(FetchMsg::Done(text)) if text == "fetched"));
    }

    #[test]
    fn a_running_job_stays_in_its_slot() {
        let (_tx, rx) = std::sync::mpsc::channel::<FetchMsg>();
        let mut slot = Some(FetchJob {
            rx,
            handle: None,
            spinner: 0,
        });

        assert!(take_finished(&mut slot).is_none());
        assert!(slot.is_some());
    }

    #[test]
    fn a_stream_whose_worker_died_mid_answer_is_ended() {
        let (rx, handle) = dying_worker::<GenMsg>();
        let mut slot = Some(ReviewChatJob {
            rx,
            handle: Some(handle),
            output: String::new(),
            spinner: 0,
        });

        let drained = drain_messages(&slot);
        let (_, status) =
            reap_stopped(&mut slot, drained.disconnected).expect("the stream is over");

        assert!(slot.is_none());
        assert!(
            status.contains("review chat stopped unexpectedly"),
            "{status}"
        );
    }

    /// The Claude CLI sends its answer and then takes seconds to exit. The
    /// answer is shown as it arrives; the screen does not wait for the exit.
    #[test]
    fn an_answer_is_taken_without_waiting_for_its_worker_to_exit() {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            tx.send(GenMsg::Done {
                text: "the explanation".into(),
                stats: crate::llm::GenStats::default(),
            })
            .unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
        });
        let mut slot = Some(ReviewAssistJob {
            rx,
            handle: Some(handle),
            node_id: "node".into(),
            output: String::new(),
            spinner: 0,
        });
        let mut assists = std::collections::HashMap::new();
        let mut deferred = Vec::new();
        let started = std::time::Instant::now();
        let mut status = None;
        while status.is_none() && started.elapsed() < std::time::Duration::from_secs(1) {
            status = drain_review_stream(&mut slot, &mut assists, &mut deferred, "ready");
        }

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "the drain waited for the worker to exit"
        );
        assert_eq!(status, Some(("ready".to_string(), false)));
        assert_eq!(
            assists.get("node").map(String::as_str),
            Some("the explanation")
        );
    }

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
