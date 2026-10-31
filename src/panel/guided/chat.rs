//! The Claude Code chat beside the walk: when it opens, what it is about,
//! and what it is told as the walk moves on.

use crate::state::{AppState, Modal, PendingAction};

use super::found::ticket_in;
use super::{ChatNews, Guided, Mode, WHOLE_CHANGE, draw, findings};

impl Guided {
    /// What a chat opened now is about: the file on screen, or on the
    /// overview and the final step [`WHOLE_CHANGE`].
    pub fn chat_subject(&self) -> Option<String> {
        Some(match self.step() {
            Some(step) => step.path.clone(),
            None => WHOLE_CHANGE.to_string(),
        })
    }

    pub fn chat_brief(&self, path: &str) -> String {
        if path == WHOLE_CHANGE {
            let mut out = String::from("Open across the whole change, worst first:\n");
            for open in self.open_findings() {
                let path = &self.steps[open.step - 1].path;
                let from = if open.whole {
                    " (from the review of the whole change)"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "- {path}:L{} {}{from}: {}\n",
                    open.finding.line,
                    open.finding.severity.word(),
                    open.finding.text
                ));
            }
            for note in &self.progress.notes {
                out.push_str(&format!(
                    "- {}:L{} note: {}\n",
                    note.path, note.line, note.body
                ));
            }
            return out;
        }
        let mut out = format!("Open in {path}:\n");
        for step in self.steps.iter().filter(|step| step.path == path) {
            let Some(commentary) = self.commentary.get(&step.key()).filter(|c| c.done) else {
                continue;
            };
            let reading = findings::read(&commentary.text);
            for fixed in &reading.fixed {
                out.push_str(&format!("- fixed: {fixed}\n"));
            }
            let open = reading
                .findings
                .into_iter()
                .filter(|finding| !self.is_ticked(&ticket_in(path, finding)));
            for finding in open {
                let word = finding.severity.word();
                out.push_str(&format!("- L{} {word}: {}\n", finding.line, finding.text));
            }
        }
        let across = if self.final_done() {
            self.final_cited()
        } else {
            Vec::new()
        };
        let across = across
            .into_iter()
            .filter(|(cited, finding)| cited == path && !self.is_ticked(&ticket_in(path, finding)));
        for (_, finding) in across {
            let word = finding.severity.word();
            out.push_str(&format!(
                "- L{} {word} (from the review of the whole change): {}\n",
                finding.line, finding.text
            ));
        }
        for note in self.progress.notes.iter().filter(|note| note.path == path) {
            out.push_str(&format!("- L{} note: {}\n", note.line, note.body));
        }
        out
    }

    /// A file the change no longer touches has nothing left to wait for.
    pub fn has_read(&self, path: &str) -> bool {
        if path == WHOLE_CHANGE {
            return self.loading.is_none() && self.final_done();
        }
        self.loading.is_none()
            && self
                .steps
                .iter()
                .filter(|step| step.path == path)
                .all(|step| self.commentary.get(&step.key()).is_some_and(|c| c.done))
    }

    pub fn owe_chat(&mut self, path: String, news: ChatNews) {
        match self.chat_owed.iter_mut().find(|(owed, _)| *owed == path) {
            Some((_, owed)) => *owed = (*owed).max(news),
            None => self.chat_owed.push((path, news)),
        }
    }
}

pub(super) fn note_file_change_for_chat(guided: &mut Guided) {
    let Some(path) = guided.chat_subject() else {
        return;
    };
    if guided.chat.is_none() || guided.chat_file.as_deref() == Some(path.as_str()) {
        return;
    }
    guided.chat_file = Some(path.clone());
    guided.owe_chat(path, ChatNews::FileRead);
}

/// Each piece of news waits until lg has read the whole file it is about.
pub(super) fn tell_chat_what_is_read(guided: &mut Guided, sessions: &mut crate::session::Sessions) {
    let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut guided.chat_owed)
        .into_iter()
        .partition(|(path, _)| guided.has_read(path));
    guided.chat_owed = waiting;
    let Some(session) = guided.chat.and_then(|id| sessions.get_mut(id)) else {
        return;
    };
    for (path, news) in ready {
        let brief = guided.chat_brief(&path);
        let message = match news {
            ChatNews::FileRead => crate::llm::build_guided_chat_file_read(&brief),
            ChatNews::Redone => crate::llm::build_guided_chat_redo(&brief),
        };
        let bracketed = session.bracketed_paste();
        session.send(&crate::term::encode_paste(&message, bracketed));
        session.send(b"\r");
    }
}

pub(super) fn open_chat(state: &mut AppState) {
    let running = chat_session(state);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !guided.source.editable() {
        state.set_status(
            "this pull request is not checked out here \u{2014} check it out to fix it in a chat",
            false,
        );
        return;
    }
    if running.is_some() {
        guided.mode = Mode::Chat;
        state.set_status("chat \u{2014} Ctrl-] back to the review", false);
        return;
    }
    state.pending_action = Some(PendingAction::GuidedChat { focus: true });
}

/// Hand the chat what to fix, in its input for the reviewer to read and
/// send: the note or findings on the cursor line, the picked finding on the
/// final step, or on the overview everything still open.
pub(super) fn fix_in_chat(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    match guided.fix_request() {
        Some(request) => send_to_chat(state, &request),
        None => state.set_status("nothing flagged here to fix", false),
    }
}

impl Guided {
    /// What `f` asks the chat to fix where the reviewer is.
    pub(super) fn fix_request(&self) -> Option<String> {
        let guided = self;
        if guided.on_final() {
            guided
                .picked()
                .map(|(path, finding)| format!("Fix {path}:L{}: {}", finding.line, finding.text))
        } else if let Some(step) = guided.step() {
            let text = guided
                .note_at_cursor()
                .and_then(|index| guided.progress.notes.get(index))
                .map(|note| note.body.clone())
                .unwrap_or_else(|| guided.findings_at_cursor());
            let line = step
                .lines
                .get(guided.cursor)
                .and_then(|line| line.new.or(line.old));
            (!text.trim().is_empty()).then(|| match line {
                Some(n) => format!("Fix {}:L{n}: {}", step.path, text.trim()),
                None => format!("Fix {}: {}", step.path, text.trim()),
            })
        } else {
            (!guided.open_findings().is_empty()).then(|| {
                format!(
                    "Fix what is open, one at a time, worst first.\n{}",
                    guided.chat_brief(WHOLE_CHANGE)
                )
            })
        }
    }
}

/// Puts `text` in the chat's input and gives it the keyboard, so the
/// reviewer can add to it before sending; starts the chat first if needed.
pub(super) fn send_to_chat(state: &mut AppState, text: &str) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if !guided.source.editable() {
        state.set_status(
            "this pull request is not checked out here \u{2014} check it out to fix it in a chat",
            false,
        );
        return;
    }
    let Some(id) = chat_session(state) else {
        state.pending_action = Some(PendingAction::GuidedChat { focus: true });
        state.set_status(
            "starting Claude \u{2014} press f again once it is up",
            false,
        );
        return;
    };
    if let Some(session) = state.sessions.get_mut(id) {
        let bracketed = session.bracketed_paste();
        session.send(&crate::term::encode_paste(text, bracketed));
    }
    if let Some(guided) = state.guided.as_mut() {
        guided.mode = Mode::Chat;
    }
    state.set_status(
        "in the chat \u{2014} Enter sends it, Ctrl-] back to the review",
        false,
    );
}

pub(super) fn chat_session(state: &AppState) -> Option<crate::session::SessionId> {
    state
        .guided
        .as_ref()?
        .chat
        .filter(|id| state.sessions.get(*id).is_some())
}

pub fn chat_on_screen(state: &AppState) -> Option<crate::session::SessionId> {
    (state.modal == Modal::GuidedReview)
        .then(|| chat_session(state))
        .flatten()
}

/// Then every key, Ctrl-C and Esc included, goes to Claude.
pub fn chat_has_keyboard(state: &AppState) -> bool {
    state.modal == Modal::GuidedReview
        && state
            .guided
            .as_ref()
            .is_some_and(|guided| guided.mode == Mode::Chat)
}

pub fn fit_chat(state: &mut AppState, area: ratatui::layout::Rect) {
    if state.modal != Modal::GuidedReview {
        return;
    }
    let Some(id) = chat_session(state) else {
        return;
    };
    let mode = state.diff_view_mode;
    let Some(screen) = state.guided.as_deref().and_then(|guided| {
        let (_, side) = draw::pane_areas(guided, area, state.guided_list_width)?;
        draw::chat_screen_area(guided, side, mode)
    }) else {
        return;
    };
    if let Some(session) = state.sessions.get_mut(id) {
        session.resize(screen.height, screen.width);
    }
}
