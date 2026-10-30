//! The reviewer's notes: writing them on lines, the summary of all of them,
//! and for a pull request submitting them as one review.

use super::*;

/// Save the note being written on the cursor line.
pub(super) fn commit_note(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Mode::Note { text, editing } = std::mem::take(&mut guided.mode) else {
        return;
    };
    let body = text.trim().to_string();
    match editing {
        Some(index) if body.is_empty() => {
            guided.progress.notes.remove(index);
        }
        Some(index) => {
            if let Some(note) = guided.progress.notes.get_mut(index) {
                note.body = body;
            }
        }
        None if body.is_empty() => return,
        None => {
            let Some(step) = guided.step() else {
                state.set_status("notes go on a file's lines", false);
                return;
            };
            let Some(line) = step.lines.get(guided.cursor) else {
                return;
            };
            let Some((number, side)) = line.anchor() else {
                state.set_status("move to a code line to pin the note to", false);
                return;
            };
            let note = GuidedNote {
                path: step.path.clone(),
                line: number,
                side,
                body,
                step: step.key(),
                quote: line.text.clone(),
            };
            guided.progress.notes.push(note);
        }
    }
    let count = guided.progress.notes.len();
    match guided.save() {
        Ok(()) => state.set_status(format!("{count} note(s) in this review"), false),
        Err(message) => state.set_status(message, true),
    }
}

pub(super) fn delete_note_at_cursor(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let armed = guided.delete_armed.take();
    let Some(index) = guided.note_at_cursor() else {
        state.set_status("no note on this line", false);
        return;
    };
    // A note is the one thing the walk cannot give back, so the first press
    // only asks.
    if armed != Some(index) {
        guided.delete_armed = Some(index);
        state.set_status("x again deletes the note on this line", false);
        return;
    }
    guided.progress.notes.remove(index);
    match guided.save() {
        Ok(()) => state.set_status("note deleted", false),
        Err(message) => state.set_status(message, true),
    }
}

pub(super) fn open_summary(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    guided.mode = Mode::Summary {
        event: 0,
        body: TextInput::default(),
    };
    guided.side_scroll = 0;
}

/// Submit the notes as one review of the pull request.
pub(super) fn submit(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Source::PullRequest { number, commit, .. } = &guided.source else {
        return;
    };
    let Mode::Summary { event, body } = std::mem::take(&mut guided.mode) else {
        return;
    };
    let event = crate::github::ReviewEvent::ALL[event % crate::github::ReviewEvent::ALL.len()];
    let shown: HashSet<(String, usize, Side)> = guided
        .steps
        .iter()
        .flat_map(|step| {
            step.lines
                .iter()
                .filter_map(|line| line.anchor())
                .map(|(n, side)| (step.path.clone(), n, side))
        })
        .collect();
    let (comments, stale): (Vec<_>, Vec<_>) = guided
        .progress
        .notes
        .iter()
        .partition(|note| shown.contains(&(note.path.clone(), note.line, note.side)));
    if comments.is_empty() && body.trim().is_empty() && event != crate::github::ReviewEvent::Approve
    {
        guided.mode = Mode::Summary {
            event: crate::github::ReviewEvent::ALL
                .iter()
                .position(|e| *e == event)
                .unwrap_or(0),
            body,
        };
        state.set_status("nothing to submit: write a note or a summary first", false);
        return;
    }
    let mut body = body.trim().to_string();
    if !stale.is_empty() {
        // A note whose line the diff no longer shows cannot be pinned, so it
        // goes in the body rather than being dropped.
        body.push_str("\n\n");
        for note in &stale {
            body.push_str(&format!("- `{}:{}` {}\n", note.path, note.line, note.body));
        }
    }
    let comments = comments
        .into_iter()
        .map(|note| crate::github::LineComment {
            path: note.path.clone(),
            line: note.line,
            side: note.side.github(),
            body: note.body.clone(),
        })
        .collect();
    state.pending_action = Some(PendingAction::GitHub(
        crate::state::GitHubAction::SubmitReview {
            number: *number,
            commit: commit.clone(),
            event,
            body: body.trim().to_string(),
            comments,
        },
    ));
}

/// The review went to GitHub: its notes are there now, and sending them again
/// would post every comment twice. A failed submit keeps them to try again.
pub fn after_submit(state: &mut AppState, succeeded: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !succeeded || !guided.source.is_pull_request() {
        return;
    }
    guided.progress.notes.clear();
    if let Err(message) = guided.save() {
        state.set_status(message, true);
    }
}

/// Every note as Markdown, file by file, for the clipboard or a fix request.
pub(super) fn notes_markdown(guided: &Guided) -> String {
    let mut notes: Vec<&GuidedNote> = guided.progress.notes.iter().collect();
    notes.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    let mut out = format!("Review notes: {}\n", guided.source.title());
    let mut path = "";
    for note in notes {
        if note.path != path {
            path = &note.path;
            out.push_str(&format!("\n## {path}\n"));
        }
        let side = if note.side == Side::Left {
            " (removed line)"
        } else {
            ""
        };
        out.push_str(&format!("- L{}{side}: {}\n", note.line, note.body));
        let quote = note.quote.trim();
        if !quote.is_empty() {
            out.push_str(&format!("  > `{quote}`\n"));
        }
    }
    out
}
