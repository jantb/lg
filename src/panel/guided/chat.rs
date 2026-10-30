//! The Claude Code chat beside the walk: when it opens, what it is about,
//! and what it is told as the walk moves on.

use super::*;

impl Guided {
    /// What a chat opened now is about: the file on screen, or on the final
    /// step [`WHOLE_CHANGE`].
    pub fn chat_subject(&self) -> Option<String> {
        match self.step() {
            Some(step) => Some(step.path.clone()),
            None if self.on_final() => Some(WHOLE_CHANGE.to_string()),
            None => None,
        }
    }

    pub fn chat_brief(&self, path: &str) -> String {
        if path == WHOLE_CHANGE {
            let mut out = format!("Open across the whole change:\n{}", self.final_open_lines());
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
    if guided.chat_subject().is_none() {
        state.set_status(
            "the chat is about a file or the whole change: \u{2192} to one first",
            false,
        );
        return;
    }
    state.pending_action = Some(PendingAction::GuidedChat);
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
