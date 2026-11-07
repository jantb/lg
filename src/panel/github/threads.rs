//! Going through a pull request's conversations: the review threads still
//! open, one after the other, replying to each and resolving it, with a count
//! of how many are left.
//!
//! A reply or a resolution shows at once and goes to GitHub on a worker of its
//! own, so the next conversation is there to read while the last one is still
//! being sent. Once every write is answered the conversations are read again,
//! and a write that failed is undone by that read.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::github::{ReviewThread, ThreadComment, ThreadCount};
use crate::panel::text_input::TextInput;
use crate::state::{AppState, GitHubAction, Poll, poll_once};

use super::browse::dispatch;
use super::work::{Pending, spawn_read, take_answer};
use super::{GitHub, Mode};

/// The conversations of one pull request, and where the user is among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadsView {
    pub number: u64,
    pub title: String,
    pub threads: Vec<ReviewThread>,
    /// Whether they have been read at least once.
    pub loaded: bool,
    pub error: Option<String>,
    /// The selected thread, among those [`visible`] lists.
    pub idx: usize,
    /// Resolved conversations are left out unless asked for: what is left to
    /// do is what the view is for.
    pub show_resolved: bool,
    /// How far the selected conversation is scrolled.
    pub scroll: u16,
    /// The reply being written to the selected conversation, if one is.
    pub reply: Option<TextInput>,
}

impl ThreadsView {
    fn new(number: u64, title: String) -> Self {
        Self {
            number,
            title,
            threads: Vec::new(),
            loaded: false,
            error: None,
            idx: 0,
            show_resolved: false,
            scroll: 0,
            reply: None,
        }
    }

    /// The indices into `threads` of the conversations shown, in order.
    pub fn visible(&self) -> Vec<usize> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, t)| self.show_resolved || !t.resolved)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected(&self) -> Option<&ReviewThread> {
        let i = *self.visible().get(self.idx)?;
        self.threads.get(i)
    }

    fn selected_mut(&mut self) -> Option<&mut ReviewThread> {
        let i = *self.visible().get(self.idx)?;
        self.threads.get_mut(i)
    }

    pub fn count(&self) -> ThreadCount {
        ThreadCount::of(&self.threads)
    }

    fn select(&mut self, idx: usize) {
        let last = self.visible().len().saturating_sub(1);
        self.idx = idx.min(last);
        self.scroll = 0;
    }

    /// Keep the selection on the thread `id` if it is still shown, or on the
    /// one that took its place.
    fn reselect(&mut self, id: Option<&str>) {
        let visible = self.visible();
        if let Some(pos) = id.and_then(|id| visible.iter().position(|&i| self.threads[i].id == id))
        {
            self.idx = pos;
        }
        self.idx = self.idx.min(visible.len().saturating_sub(1));
    }
}

/// What a finished write reports: which pull request it was on, and what to
/// say about it.
pub type Written = (u64, String);

/// Open the conversations of the selected pull request.
pub(super) fn open(state: &mut AppState, number: u64, title: String) {
    state.github.mode = Mode::Threads(ThreadsView::new(number, title));
    load(&mut state.github, number);
}

fn load(gh: &mut GitHub, number: u64) {
    gh.threads_loading = Some(spawn_read(move || {
        Ok((number, crate::github::review_threads(number)?))
    }));
}

/// Read how many conversations each listed pull request has open.
pub(super) fn load_counts(gh: &mut GitHub) {
    let numbers: Vec<u64> = gh
        .prs
        .iter()
        .filter(|pr| pr.is_open())
        .map(|pr| pr.number)
        .collect();
    if numbers.is_empty() {
        return;
    }
    gh.counts_loading = Some(spawn_read(move || crate::github::thread_counts(&numbers)));
}

fn current(state: &mut AppState) -> Option<&mut ThreadsView> {
    match &mut state.github.mode {
        Mode::Threads(view) => Some(view),
        _ => None,
    }
}

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) {
    let replying = current(state).is_some_and(|v| v.reply.is_some());
    if replying {
        reply_key(state, key);
    } else {
        browse_key(state, key);
    }
}

fn browse_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(view) = current(state) else {
        return;
    };
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => state.github.mode = Mode::Browse,
        KeyCode::Char('j') | KeyCode::Down => view.select(view.idx + 1),
        KeyCode::Char('k') | KeyCode::Up => view.select(view.idx.saturating_sub(1)),
        KeyCode::Char('g') | KeyCode::Home => view.select(0),
        KeyCode::Char('G') | KeyCode::End => view.select(usize::MAX),
        KeyCode::Char('d') if ctrl => view.scroll = view.scroll.saturating_add(5),
        KeyCode::Char('u') if ctrl => view.scroll = view.scroll.saturating_sub(5),
        KeyCode::Char('r') if ctrl => {
            let number = view.number;
            load(&mut state.github, number);
        }
        KeyCode::Char('h') => {
            let id = view.selected().map(|t| t.id.clone());
            view.show_resolved = !view.show_resolved;
            view.reselect(id.as_deref());
            view.scroll = 0;
        }
        KeyCode::Enter | KeyCode::Char('r') => {
            if view.selected().is_some() {
                view.reply = Some(TextInput::default());
            } else {
                state.set_status("no conversation selected", false);
            }
        }
        KeyCode::Char(' ') => {
            let Some(thread) = view.selected() else {
                state.set_status("no conversation selected", false);
                return;
            };
            let resolve = !thread.resolved;
            let id = thread.id.clone();
            set_resolved(state, &id, resolve);
            send(state, id, None, resolve);
        }
        KeyCode::Char('o') => match view.selected().map(|t| t.url.clone()) {
            Some(url) if !url.is_empty() => dispatch(state, GitHubAction::OpenInBrowser { url }),
            _ => state.set_status("nothing to open", false),
        },
        _ => {}
    }
}

fn reply_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let Some(view) = current(state) else {
        return;
    };
    let Some(text) = view.reply.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Esc => view.reply = None,
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Enter => {
            let body = text.trim().to_string();
            if body.is_empty() {
                state.set_status("write a reply first; Esc cancels", false);
                return;
            }
            view.reply = None;
            let Some(thread) = view.selected_mut() else {
                return;
            };
            let id = thread.id.clone();
            let resolve = alt && !thread.resolved;
            let me = state
                .github
                .owners
                .as_ref()
                .map(|o| o.login.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "you".into());
            if let Some(thread) = current(state).and_then(ThreadsView::selected_mut) {
                thread.comments.push(ThreadComment {
                    author: me,
                    body: body.clone(),
                    created_at: String::new(),
                });
            }
            if resolve {
                set_resolved(state, &id, true);
            }
            send(state, id, Some(body), resolve);
        }
        _ => {
            text.handle_key(key);
        }
    }
}

/// Text pasted into a reply, on one line as the reply is written.
pub(super) fn paste(view: &mut ThreadsView, text: &str) -> bool {
    match view.reply.as_mut() {
        Some(reply) => {
            reply.insert_str(&text.replace(['\r', '\n'], " "));
            true
        }
        None => false,
    }
}

/// Show `id` as resolved or not, here and in the count for its pull request,
/// ahead of GitHub's say-so.
fn set_resolved(state: &mut AppState, id: &str, resolved: bool) {
    let Some(view) = current(state) else {
        return;
    };
    let keep = view.selected().map(|t| t.id.clone());
    if let Some(thread) = view.threads.iter_mut().find(|t| t.id == id) {
        thread.resolved = resolved;
        if !resolved {
            thread.resolved_by = None;
        }
    }
    // A conversation resolved out of sight leaves the one after it selected,
    // which is the next to go through.
    view.reselect(keep.as_deref());
    view.scroll = 0;
    let (number, count) = (view.number, view.count());
    state.github.thread_counts.insert(number, count);
}

/// Send a reply, a resolution, or both, on a worker of their own. Without a
/// reply, `resolve` is the state the conversation is to be left in; with one,
/// whether it is resolved after the reply.
fn send(state: &mut AppState, id: String, reply: Option<String>, resolve: bool) {
    let Some(number) = current(state).map(|v| v.number) else {
        return;
    };
    let pending = spawn_write(number, id, reply, resolve);
    state.github.thread_writes.push(pending);
}

#[cfg(not(test))]
fn spawn_write(number: u64, id: String, reply: Option<String>, resolve: bool) -> Pending<Written> {
    spawn_read(move || {
        let said = match reply {
            Some(body) => {
                crate::github::reply_to_thread(&id, &body)?;
                if resolve {
                    crate::github::resolve_thread(&id, true)?;
                    "replied to and resolved"
                } else {
                    "replied to"
                }
            }
            None => {
                crate::github::resolve_thread(&id, resolve)?;
                if resolve { "resolved" } else { "reopened" }
            }
        };
        Ok((number, said.to_string()))
    })
}

/// Tests never write to GitHub: each write is recorded, and answered when the
/// test says so.
#[cfg(test)]
fn spawn_write(number: u64, id: String, reply: Option<String>, resolve: bool) -> Pending<Written> {
    tests::record_write(number, id, reply, resolve)
}

/// Take in conversations read and writes answered.
pub(super) fn poll(state: &mut AppState) {
    if let Some(result) = take_answer(&mut state.github.counts_loading) {
        match result {
            Ok(counts) => {
                // A pull request whose conversations are open here keeps the
                // count of what is on screen, which may be ahead of GitHub.
                let open = current(state).map(|v| v.number);
                for (number, count) in counts {
                    if Some(number) != open {
                        state.github.thread_counts.insert(number, count);
                    }
                }
            }
            Err(err) => state.set_status(format!("could not count conversations: {err}"), true),
        }
    }

    let mut finished = Vec::new();
    state
        .github
        .thread_writes
        .retain(|pending| match poll_once(pending) {
            Poll::Pending => true,
            Poll::Message(answer) => {
                finished.push(answer);
                false
            }
            Poll::Disconnected => {
                finished.push(Err("a write to GitHub ended without an answer".into()));
                false
            }
        });
    let mut reread = None;
    for answer in finished {
        match answer {
            Ok((number, said)) => {
                let left = state
                    .github
                    .thread_counts
                    .get(&number)
                    .map(|c| left_to_resolve(c.unresolved))
                    .unwrap_or_default();
                state.set_status(format!("{said} a conversation on #{number}{left}"), false);
                reread = Some(number);
            }
            Err(err) => {
                state.set_status(err, true);
                reread = current(state).map(|v| v.number);
            }
        }
    }
    // Read again once nothing is still on its way, so a read never undoes a
    // write that has not landed yet.
    if let Some(number) = reread
        && state.github.thread_writes.is_empty()
        && current(state).is_some_and(|v| v.number == number)
    {
        load(&mut state.github, number);
    }

    if let Some(result) = take_answer(&mut state.github.threads_loading) {
        let writing = !state.github.thread_writes.is_empty();
        let Some(view) = current(state) else {
            return;
        };
        match result {
            Ok((number, threads)) if number == view.number => {
                if writing {
                    // Older than what is on screen; the read after the writes
                    // will bring it up to date.
                    return;
                }
                let keep = view.selected().map(|t| t.id.clone());
                view.threads = threads;
                view.loaded = true;
                view.error = None;
                view.reselect(keep.as_deref());
                let count = view.count();
                state.github.thread_counts.insert(number, count);
            }
            Ok(_) => {}
            Err(err) => {
                view.error = Some(err);
                view.loaded = true;
            }
        }
    }
}

/// How many are left, for the end of a status line.
fn left_to_resolve(unresolved: usize) -> String {
    match unresolved {
        0 => " \u{b7} all resolved".into(),
        n => format!(" \u{b7} {n} left to resolve"),
    }
}

#[cfg(test)]
mod tests;
