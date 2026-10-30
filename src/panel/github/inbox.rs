//! The inbox tab: what on GitHub is waiting on the user across the workspace's
//! repositories. It is read in the background every few minutes, so the header
//! can say so before anyone opens the modal, and each thing is announced once.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::Tab;
use super::browse::{dispatch, switch_tab};
use super::work::{Pending, spawn_read, take_answer};
use crate::config::GITHUB_INBOX_INTERVAL_SECS;
use crate::github::{InboxItem, PullRequest, Reason, Section, Subject};
use crate::state::{AppState, GitHubAction, Modal, Poll, poll_once};

#[derive(Debug, Default)]
pub struct Inbox {
    pub read: crate::github::Inbox,
    pub idx: usize,
    pub error: Option<String>,
    pub loading: Option<Pending<crate::github::Inbox>>,
    last_started: Option<Instant>,
    /// What has been announced, and why, so each thing is announced once.
    announced: HashSet<(String, Reason)>,
    /// Review requests marked done here. GitHub keeps asking for the review
    /// until it is given, so lg is the one that remembers to stop showing it.
    done: HashSet<String>,
    /// Threads being marked done on GitHub, with the call doing it.
    marking: Vec<(Vec<String>, Pending<()>)>,
    /// The pull request a guided review is waiting on.
    reviewing: Option<Pending<PullRequest>>,
}

impl Inbox {
    pub fn items(&self) -> &[InboxItem] {
        &self.read.items
    }

    pub fn selected(&self) -> Option<&InboxItem> {
        self.read.items.get(self.idx)
    }

    /// The checkout a review of `item` would run in, when it is a pull
    /// request lg can review.
    pub fn review_checkout(&self, item: &InboxItem) -> Option<&PathBuf> {
        matches!(item.subject, Subject::PullRequest(_))
            .then(|| self.read.checkout(&item.repo))
            .flatten()
    }

    /// How many items fall in `section`.
    pub fn section_len(&self, section: Section) -> usize {
        self.read
            .items
            .iter()
            .filter(|item| item.reason.section() == section)
            .count()
    }

    /// How many items there are, and how many of them wait on the user by
    /// name.
    pub fn counts(&self) -> (usize, usize) {
        (self.read.items.len(), self.section_len(Section::You))
    }

    /// Whether the inbox has been read at least once, so an empty one means
    /// nothing is waiting rather than not knowing yet.
    pub fn was_read(&self) -> bool {
        self.last_started.is_some() && self.loading.is_none()
    }
}

/// The checkouts whose repositories the inbox watches: the one on screen
/// first, so a review lands there rather than in another checkout of the same
/// repository, then the workspace and the repositories nested in it.
fn watched_dirs(state: &AppState) -> Vec<PathBuf> {
    let workspace = state.workspace_root.as_deref().map(PathBuf::from);
    let nested = state
        .nested_repositories
        .iter()
        .filter(|repo| repo.worktree_of.is_none())
        .filter_map(|repo| Some(workspace.as_ref()?.join(&repo.path)));
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in state
        .repo_root
        .as_deref()
        .map(PathBuf::from)
        .into_iter()
        .chain(workspace.clone())
        .chain(nested)
    {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Read the inbox again, replacing any read still under way.
pub fn reload(state: &mut AppState) {
    let dirs = watched_dirs(state);
    let inbox = &mut state.github.inbox;
    inbox.loading = Some(spawn_read(move || crate::github::read_inbox(&dirs)));
    inbox.last_started = Some(Instant::now());
}

/// Whether the periodic read is due. The first waits for the first refresh,
/// which is what finds the repositories nested in the workspace.
fn due(state: &AppState) -> bool {
    let inbox = &state.github.inbox;
    inbox.loading.is_none()
        && state.refresh_job.is_none()
        && inbox.last_started.is_none_or(|started| {
            started.elapsed() >= Duration::from_secs(GITHUB_INBOX_INTERVAL_SECS)
        })
}

pub fn poll(state: &mut AppState) {
    if due(state) {
        reload(state);
    }
    if let Some(answer) = take_answer(&mut state.github.inbox.loading) {
        match answer {
            Ok(read) => take_read(state, read),
            // Kept for the tab to show: a banner every few minutes for a
            // missing gh or a lapsed login would be noise.
            Err(err) => state.github.inbox.error = Some(err),
        }
    }
    poll_marking(state);
    // A review opened before the switch to its checkout has been read would
    // judge the pull request against the branch of the checkout left behind.
    if state.refresh_job.is_none()
        && let Some(answer) = take_answer(&mut state.github.inbox.reviewing)
    {
        match answer {
            Ok(pr) => crate::panel::guided::open_pull_request(state, &pr),
            Err(err) => state.set_status(err, true),
        }
    }
}

fn take_read(state: &mut AppState, mut read: crate::github::Inbox) {
    let inbox = &mut state.github.inbox;
    let in_flight: HashSet<&str> = inbox
        .marking
        .iter()
        .flat_map(|(threads, _)| threads.iter().map(String::as_str))
        .collect();
    read.items.retain(|item| match &item.thread {
        Some(thread) => !in_flight.contains(thread.as_str()),
        None => !inbox.done.contains(&item.key()),
    });
    let selected = inbox.selected().map(InboxItem::key);
    inbox.read = read;
    inbox.idx = selected
        .and_then(|key| inbox.read.items.iter().position(|item| item.key() == key))
        .unwrap_or(inbox.idx)
        .min(inbox.read.items.len().saturating_sub(1));
    inbox.error = None;
    announce(state);
}

/// Say what has newly arrived, once.
fn announce(state: &mut AppState) {
    let inbox = &mut state.github.inbox;
    let fresh: Vec<&InboxItem> = inbox
        .read
        .items
        .iter()
        .filter(|item| !inbox.announced.contains(&(item.key(), item.reason.clone())))
        .collect();
    let text = match fresh.as_slice() {
        [] => None,
        [item] => Some(format!(
            "GitHub: {} {} \u{201c}{}\u{201d} \u{2014} H to see it",
            item.reason.label(),
            item.place(),
            item.title
        )),
        more => Some(format!(
            "GitHub: {} new things in your inbox \u{2014} H to see them",
            more.len()
        )),
    };
    inbox.announced.extend(
        inbox
            .read
            .items
            .iter()
            .map(|item| (item.key(), item.reason.clone())),
    );
    if let Some(text) = text {
        state.set_status(text, false);
    }
}

fn poll_marking(state: &mut AppState) {
    let mut failed = Vec::new();
    state
        .github
        .inbox
        .marking
        .retain(|(_, pending)| match poll_once(pending) {
            Poll::Pending => true,
            Poll::Message(Ok(())) => false,
            Poll::Message(Err(err)) => {
                failed.push(err);
                false
            }
            Poll::Disconnected => {
                failed.push("the request to GitHub ended without an answer".to_string());
                false
            }
        });
    if let Some(err) = failed.pop() {
        state.set_status(format!("could not mark it done on GitHub: {err}"), true);
        reload(state);
    }
}

/// Take the selected item out of the inbox, here and on GitHub.
fn mark_done(state: &mut AppState) {
    let inbox = &mut state.github.inbox;
    if inbox.idx >= inbox.read.items.len() {
        state.set_status("nothing to mark done", false);
        return;
    }
    let item = inbox.read.items.remove(inbox.idx);
    inbox.idx = inbox.idx.min(inbox.read.items.len().saturating_sub(1));
    if matches!(
        item.reason,
        Reason::ReviewRequested | Reason::TeamReviewRequested
    ) {
        inbox.done.insert(item.key());
    }
    if let Some(thread) = item.thread.clone() {
        let call = thread.clone();
        inbox.marking.push((
            vec![thread],
            spawn_read(move || crate::github::mark_done(&call)),
        ));
    }
    state.set_status(format!("done with {}", item.place()), false);
}

/// Take every update — what only tells the user something happened — out of
/// the inbox at once, here and on GitHub, leaving what waits on them.
fn clear_updates(state: &mut AppState) {
    let inbox = &mut state.github.inbox;
    let selected = inbox.selected().map(InboxItem::key);
    let (asking, updates): (Vec<InboxItem>, Vec<InboxItem>) = std::mem::take(&mut inbox.read.items)
        .into_iter()
        .partition(|item| item.reason.section() != Section::Updates);
    inbox.read.items = asking;
    inbox.idx = selected
        .and_then(|key| inbox.read.items.iter().position(|item| item.key() == key))
        .unwrap_or(0);
    if updates.is_empty() {
        state.set_status("no updates to clear", false);
        return;
    }
    let threads: Vec<String> = updates
        .iter()
        .filter_map(|item| item.thread.clone())
        .collect();
    let call = threads.clone();
    inbox.marking.push((
        threads,
        spawn_read(move || crate::github::mark_all_done(&call)),
    ));
    state.set_status(format!("cleared {} updates", updates.len()), false);
}

/// Review the selected pull request in its checkout, or open anything else in
/// the browser.
fn open(state: &mut AppState) {
    let inbox = &state.github.inbox;
    let Some(item) = inbox.selected() else {
        state.set_status("nothing in the inbox", false);
        return;
    };
    let action = match (inbox.review_checkout(item), &item.subject) {
        (Some(dir), Subject::PullRequest(number)) => GitHubAction::ReviewInCheckout {
            dir: dir.clone(),
            number: *number,
        },
        _ => GitHubAction::OpenInBrowser {
            url: item.url.clone(),
        },
    };
    if matches!(action, GitHubAction::ReviewInCheckout { .. }) {
        state.modal = Modal::None;
    }
    dispatch(state, action);
}

/// Fetch pull request `number` in the checkout lg now points at, and open a
/// guided review of it once it is here.
pub fn start_review(state: &mut AppState, number: u64) {
    state.github.inbox.reviewing = Some(spawn_read(move || crate::github::pull_request(number)));
    state.set_status(
        format!("reading #{number} for a guided review\u{2026}"),
        false,
    );
}

pub(super) fn select(state: &mut AppState, idx: usize) {
    let inbox = &mut state.github.inbox;
    inbox.idx = idx.min(inbox.read.items.len().saturating_sub(1));
}

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) {
    let idx = state.github.inbox.idx;
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Tab => switch_tab(state, true),
        KeyCode::BackTab => switch_tab(state, false),
        KeyCode::Char('j') | KeyCode::Down => select(state, idx + 1),
        KeyCode::Char('k') | KeyCode::Up => select(state, idx.saturating_sub(1)),
        KeyCode::Char('g') | KeyCode::Home => select(state, 0),
        KeyCode::Char('G') | KeyCode::End => select(state, usize::MAX),
        KeyCode::Enter | KeyCode::Char('v') => open(state),
        KeyCode::Char('d') => mark_done(state),
        KeyCode::Char('D') => clear_updates(state),
        KeyCode::Char('o') => match state.github.inbox.selected() {
            Some(item) => {
                let url = item.url.clone();
                dispatch(state, GitHubAction::OpenInBrowser { url });
            }
            None => state.set_status("nothing to open", false),
        },
        KeyCode::Char('r') => reload(state),
        _ => {}
    }
}

/// The tab the modal opens on: the inbox while something is waiting.
pub(super) fn opening_tab(state: &AppState) -> Option<Tab> {
    (!state.github.inbox.items().is_empty()).then_some(Tab::Inbox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PendingAction;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(state: &mut AppState, c: char) {
        crate::panel::github::handle_key(
            state,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .unwrap();
    }

    fn review_request(repo: &str, number: u64) -> InboxItem {
        InboxItem {
            repo: repo.into(),
            subject: Subject::PullRequest(number),
            title: format!("Change {number}"),
            reason: Reason::ReviewRequested,
            url: format!("https://github.com/{repo}/pull/{number}"),
            thread: None,
            updated_at: String::new(),
        }
    }

    fn issue_mention(repo: &str, number: u64) -> InboxItem {
        InboxItem {
            subject: Subject::Issue(number),
            reason: Reason::Mentioned,
            url: format!("https://github.com/{repo}/issues/{number}"),
            thread: Some(format!("t{number}")),
            ..review_request(repo, number)
        }
    }

    fn read(items: Vec<InboxItem>) -> crate::github::Inbox {
        crate::github::Inbox {
            items,
            checkouts: [("acme/app".to_string(), PathBuf::from("/work/app"))].into(),
        }
    }

    fn inbox_open_on(items: Vec<InboxItem>) -> AppState {
        let mut state = AppState::new();
        take_read(&mut state, read(items));
        crate::panel::github::open(&mut state);
        state
    }

    fn github_action(state: &AppState) -> Option<GitHubAction> {
        match &state.pending_action {
            Some(PendingAction::GitHub(action)) => Some(action.clone()),
            _ => None,
        }
    }

    #[test]
    fn the_modal_opens_on_the_inbox_while_something_is_waiting() {
        let state = inbox_open_on(vec![review_request("acme/app", 7)]);

        assert_eq!(state.github.tab, Tab::Inbox);
    }

    #[test]
    fn a_review_request_opens_a_guided_review_in_its_checkout() {
        let mut state = inbox_open_on(vec![review_request("acme/app", 7)]);

        press(&mut state, 'v');

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::ReviewInCheckout {
                dir: PathBuf::from("/work/app"),
                number: 7,
            })
        );
    }

    #[test]
    fn what_is_not_a_pull_request_opens_in_the_browser() {
        let mut state = inbox_open_on(vec![issue_mention("acme/app", 3)]);

        press(&mut state, 'v');

        assert_eq!(
            github_action(&state),
            Some(GitHubAction::OpenInBrowser {
                url: "https://github.com/acme/app/issues/3".into(),
            })
        );
    }

    #[test]
    fn a_review_request_marked_done_stays_done_when_github_still_asks() {
        let mut state = inbox_open_on(vec![review_request("acme/app", 7)]);

        press(&mut state, 'd');
        take_read(&mut state, read(vec![review_request("acme/app", 7)]));

        assert!(state.github.inbox.items().is_empty());
    }

    #[test]
    fn clearing_updates_keeps_what_waits_on_the_user() {
        let comment = InboxItem {
            reason: Reason::Commented,
            thread: Some("t9".into()),
            ..review_request("acme/app", 9)
        };
        let mut state = inbox_open_on(vec![review_request("acme/app", 7), comment.clone()]);

        press(&mut state, 'D');

        assert_eq!(state.github.inbox.items(), &[review_request("acme/app", 7)]);
        take_read(
            &mut state,
            read(vec![review_request("acme/app", 7), comment]),
        );
        assert_eq!(
            state.github.inbox.items().len(),
            1,
            "a thread still being marked done stays hidden"
        );
    }

    #[test]
    fn something_new_is_announced_once() {
        let mut state = AppState::new();
        take_read(&mut state, read(vec![review_request("acme/app", 7)]));
        assert!(
            state
                .status
                .as_ref()
                .is_some_and(|status| status.text.contains("acme/app#7"))
        );

        state.status = None;
        take_read(&mut state, read(vec![review_request("acme/app", 7)]));

        assert!(state.status.is_none());
    }

    #[test]
    fn the_header_counts_what_is_waiting() {
        let state = inbox_open_on(vec![
            review_request("acme/app", 7),
            issue_mention("acme/app", 3),
        ]);

        assert_eq!(state.github.inbox.counts(), (2, 2));
    }
}
