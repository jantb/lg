//! The reads from GitHub that run off the UI thread, each answering once on a
//! channel of its own.

use std::sync::mpsc::Receiver;

use crate::github::{Owners, PrFilter, PullRequest, RepoInfo, Repository};
use crate::state::{Poll, poll_once};

/// A list on its way from GitHub.
pub type Pending<T> = Receiver<Result<T, String>>;

/// Run `read` on a worker pinned to the selected checkout. Left to finish on
/// its own if replaced: it only reads, and its answer is dropped along with
/// the channel.
fn spawn_read<T, F>(read: F) -> Pending<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    drop(crate::git::spawn_pinned(move || {
        let _ = tx.send(read().map_err(|err| format!("{err:#}")));
    }));
    rx
}

/// The pull requests `filter` selects, with the repository's details when
/// `need_info` asks for them.
pub(super) fn pull_requests(
    filter: PrFilter,
    need_info: bool,
) -> Pending<(Option<RepoInfo>, Vec<PullRequest>)> {
    spawn_read(move || {
        let info = if need_info {
            Some(crate::github::repo_info()?)
        } else {
            None
        };
        Ok((info, crate::github::pull_requests(filter)?))
    })
}

pub(super) fn repositories(owner: Option<String>) -> Pending<Vec<Repository>> {
    spawn_read(move || crate::github::repositories(owner.as_deref()))
}

pub(super) fn owners() -> Pending<Owners> {
    spawn_read(crate::github::owners)
}

/// The answer in `slot`, once there is one; the slot is emptied when it is
/// taken. A worker that ended without answering answers with an error.
pub(super) fn take_answer<T>(slot: &mut Option<Pending<T>>) -> Option<Result<T, String>> {
    let answer = match poll_once(slot.as_ref()?) {
        Poll::Pending => return None,
        Poll::Message(answer) => answer,
        Poll::Disconnected => Err("the request to GitHub ended without an answer".into()),
    };
    *slot = None;
    Some(answer)
}
