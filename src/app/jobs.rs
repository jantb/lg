//! Starting the app's background jobs and taking in what they send back.

use std::thread::JoinHandle;

use crate::state::{
    BackgroundJob, Drained, Modal, ReadsCheckout, drain_receiver, stopped_unexpectedly,
};

mod conflict;
mod drain;
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

/// The message a job ended with, or the status saying its worker died first.
type Ended<J> = Result<<J as BackgroundJob>::Msg, String>;

/// [`take_finished`] for a job that read one checkout. What it read comes back
/// `None` when lg has moved to another checkout since: it is about the one
/// left behind, and applying it would show — and act on — the wrong files.
fn take_finished_here<J: BackgroundJob + ReadsCheckout>(
    slot: &mut Option<J>,
) -> Option<(J, Option<Ended<J>>)> {
    let (job, msg) = take_finished(slot)?;
    let here = job.is_for_current_checkout();
    Some((job, here.then_some(msg)))
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
    use crate::state::{FetchJob, FetchMsg, RefreshJob, RefreshMsg, RefreshScope, RefreshSnapshot};
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

    fn finished_refresh_in(checkout: &str) -> Option<RefreshJob> {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(RefreshMsg::Done(Box::new(RefreshSnapshot {
            scope: RefreshScope::Repository,
            repo_root: Some(checkout.into()),
            workspace_root: None,
            files: Some(Vec::new()),
            branches: None,
            remote_branches: None,
            nested_repositories: None,
            worktrees: None,
            release_branches: Default::default(),
            commits: None,
            unpushed_shas: None,
            branch: None,
            remote_url: None,
            ahead_behind: None,
            commit_author: None,
            decorative_animations: true,
            errors: Vec::new(),
        })))
        .unwrap();
        Some(RefreshJob {
            rx,
            handle: None,
            spinner: 0,
            refresh_diff: true,
            scope: RefreshScope::Repository,
            checkout: Some(checkout.into()),
        })
    }

    /// A refresh still reading the checkout lg just left must not put that
    /// checkout's files and branches on screen for the one it moved to.
    #[test]
    fn what_a_job_read_in_a_checkout_left_behind_is_not_handed_over() {
        let mut slot = finished_refresh_in("/work/a");

        let (_, read) = crate::git::with_repo("/work/b", || take_finished_here(&mut slot))
            .expect("the job is over");

        assert!(read.is_none(), "nothing from /work/a to apply");
        assert!(slot.is_none(), "the slot is free for the next job");
    }

    #[test]
    fn what_a_job_read_in_the_current_checkout_is_handed_over() {
        let mut slot = finished_refresh_in("/work/a");

        let (_, read) = crate::git::with_repo("/work/a", || take_finished_here(&mut slot))
            .expect("the job is over");

        assert!(matches!(read, Some(Ok(RefreshMsg::Done(_)))));
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
}
