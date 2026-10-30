//! Asking the model about a step or to read it again, and having Claude or
//! the editor change the code.

use super::*;

/// Ask the model about the step on screen.
pub(super) fn ask_question(state: &mut AppState) {
    if !state.ai_assist {
        state.set_status(crate::panel::commit::AI_OFF_NOTICE, false);
        return;
    }
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Mode::Question { text } = std::mem::take(&mut guided.mode) else {
        return;
    };
    let question = text.trim().to_string();
    if question.is_empty() {
        return;
    }
    let key = guided.step_key();
    let context = if guided.current == OVERVIEW {
        guided.overview.clone()
    } else if guided.on_final() {
        final_context(guided)
    } else {
        step_context(guided, guided.current)
    };
    let commentary = guided
        .commentary
        .get(&key)
        .filter(|c| c.done)
        .map(|c| c.text.clone())
        .unwrap_or_default();
    let exchanges = guided.exchanges.entry(key.clone()).or_default();
    exchanges.push(Exchange {
        question: question.clone(),
        answer: Commentary::default(),
    });
    let at = exchanges.len() - 1;
    let rx = work::ask_question(context, commentary, question);
    guided.asks.push(Ask {
        target: Target::Answer(key, at),
        rx,
    });
    guided.side_scroll = u16::MAX;
}

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
    guided.asks.retain(|ask| match &ask.target {
        Target::Commentary(key) => !keys.contains(key),
        Target::Answer(..) => true,
    });
    guided.checks.retain(|check| !keys.contains(&check.key));
    guided.check_queue.retain(|key| !keys.contains(key));
    for key in keys {
        guided.commentary.remove(key);
        guided.failed.remove(key);
    }
}

/// Have Claude Code make the change the reviewer described, in the checkout.
pub(super) fn start_fix(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Mode::Fix { text, all_notes } = std::mem::take(&mut guided.mode) else {
        return;
    };
    if guided.fixing.is_some() {
        state.set_status("claude is still applying the last fix", false);
        return;
    }
    let Some(root) = state.repo_root.clone() else {
        state.set_status("no checkout to change", true);
        return;
    };
    let instruction = text.trim().to_string();
    let (context, what) = if all_notes && guided.on_final() {
        (
            format!(
                "{}\nOpen findings of the review of the whole change:\n{}",
                notes_markdown(guided),
                guided.final_open_lines()
            ),
            "every open finding and note".to_string(),
        )
    } else if all_notes {
        (notes_markdown(guided), "every note".to_string())
    } else {
        if instruction.is_empty() {
            state.set_status("say what to change; Esc cancels", false);
            return;
        }
        let Some((index, at)) = guided.aim() else {
            return;
        };
        let step = &guided.steps[index - 1];
        let line = step
            .lines
            .get(at)
            .and_then(|line| line.new.or(line.old))
            .map(|n| format!(" (cursor on L{n})"))
            .unwrap_or_default();
        (
            format!(
                "{}\nThe reviewer is looking at {}{line}.",
                step_context(guided, index),
                step.path
            ),
            step.path.clone(),
        )
    };
    let instruction = match (all_notes && instruction.is_empty(), guided.on_final()) {
        (true, true) => "Address every review note and open finding below.".to_string(),
        (true, false) => "Address every review note below.".to_string(),
        (false, _) => instruction,
    };
    let prompt = crate::llm::build_guided_fix_prompt(&context, &instruction);
    guided.fixing = Some((work::run_fix(root, prompt), what.clone()));
    state.set_status(format!("claude is changing {what}\u{2026}"), false);
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
