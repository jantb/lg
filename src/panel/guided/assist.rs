//! Asking the model to read a step again, and opening the code in the editor.

use crate::state::{AppState, PendingAction};

use super::load::reload;
use super::{ChatNews, Guided, OVERVIEW_KEY, WHOLE_CHANGE};

/// Hands the model its earlier read of the file, so it can say what the edit fixed.
pub(super) fn review_again(state: &mut AppState) {
    if !state.ai_assist {
        state.set_status(crate::panel::commit::AI_OFF_NOTICE, false);
        return;
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(step) = guided.step() else {
        let key = if guided.on_final() {
            if guided.chat.is_some() {
                guided.owe_chat(WHOLE_CHANGE.to_string(), ChatNews::Redone);
            }
            guided.final_key()
        } else {
            OVERVIEW_KEY.to_string()
        };
        forget(guided, &[key]);
        reload(state);
        return;
    };
    let (path, key) = (step.path.clone(), step.key());
    if let Some(commentary) = guided.commentary.get(&key).filter(|c| c.done) {
        guided
            .earlier
            .insert(path.clone(), commentary.text.trim().to_string());
    }
    forget(guided, &[key]);
    if guided.chat.is_some() {
        guided.owe_chat(path.clone(), ChatNews::Redone);
    }
    state.set_status(format!("reviewing {path} again\u{2026}"), false);
    reload(state);
}

fn forget(guided: &mut Guided, keys: &[String]) {
    guided.asks.retain(|ask| !keys.contains(&ask.key));
    guided.checks.retain(|check| !keys.contains(&check.key));
    guided.check_queue.retain(|key| !keys.contains(key));
    for key in keys {
        guided.commentary.remove(key);
        guided.failed.remove(key);
    }
}

/// Open the file at the cursor line in the terminal editor; the walk is read
/// again once the editor exits.
pub(super) fn edit_at_cursor(state: &mut AppState, in_ide: bool) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    if !guided.source.editable() {
        state.set_status(
            "this pull request is not checked out here \u{2014} check it out (Enter in GitHub) to edit it",
            false,
        );
        return;
    }
    let Some((index, at)) = guided.aim() else {
        let message = if guided.on_final() {
            "the final review has no finding in the change to edit"
        } else {
            "pick a file to edit"
        };
        state.set_status(message, false);
        return;
    };
    let step = &guided.steps[index - 1];
    if step.file_note.as_deref() == Some("deleted file") {
        state.set_status("this file is deleted on the branch", false);
        return;
    }
    let line = step.editor_line(at);
    let path = step.path.clone();
    state.pending_action = Some(if in_ide {
        PendingAction::OpenFile(path)
    } else {
        PendingAction::EditFile { path, line }
    });
}
