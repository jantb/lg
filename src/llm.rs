//! Asking the model for a commit message, a guided review, or a conflict resolution.

use std::sync::mpsc::Sender;

use crate::state::GenMsg;

mod claude;
mod diff;
mod prompt;
mod provider;
mod reply;
mod stats;
mod stream;
mod think;

pub use claude::{
    ClaudeModel, claude_models, prime_claude_models_async, run_claude_check, run_claude_fix,
    served_claude_models,
};
pub use prompt::{GIVE_UP_PHRASE, build_conflict_hunk_prompt};
pub use provider::{
    LlmProvider, api_key, available_models, clear_saved_llm_settings, config_file_display,
    current_claude_model, current_endpoint, current_model, current_provider, endpoint_for_provider,
    env_model_active, env_provider_active, prime_models_async, save_llm_settings,
};

/// What an answer says when the server stopped it at the budget rather than at
/// the end.
///
/// An answer that was cut off reads exactly like one that finished, so anything
/// a person will act on says so in the text itself rather than only in a status
/// line that has since expired.
pub const TRUNCATED_NOTE: &str = "\u{2026} [cut off at the token budget]";
pub use stats::{GenStats, LlmPhase, forget_last_stats, last_stats, phase, progress};
pub use stream::error_means_unreachable;

use prompt::{
    build_commit_prompt, build_conventions_prompt, build_guided_final_prompt,
    build_guided_overview_prompt, build_guided_question_system_prompt,
    build_guided_step_system_prompt,
};
pub use prompt::{
    build_guided_chat_file_read, build_guided_chat_prompt, build_guided_chat_questions,
    build_guided_chat_redo, build_guided_fix_prompt, build_guided_question_check_prompt,
};
use reply::{finalize, finalize_conflict_hunk, finalize_review_chat, parse_conventions};
use stream::{ChatMessage, ChatTask, stream_messages, stream_prompt};
use think::strip_think_tags;

/// The commit message is written without a budget. A message cut off at one
/// reads as a finished message and gets committed as it stands, and the length
/// worth enforcing is the subject and body limits this checkout configures —
/// applied to a whole message afterwards, not a token count the server stops
/// mid-word on. Reasoning a model does anyway spends from the same budget,
/// which is how a short message ran out of one. Zero leaves the cap off the
/// request.
const COMMIT_NUM_PREDICT: i32 = 0;
const CONVENTIONS_NUM_PREDICT: i32 = 300;
const GUIDED_STEP_NUM_PREDICT: i32 = 2_048;
const GUIDED_OVERVIEW_NUM_PREDICT: i32 = 2_048;
const GUIDED_FINAL_NUM_PREDICT: i32 = 2_048;
const GUIDED_QUESTION_NUM_PREDICT: i32 = 1_024;
/// Enough for a conflict the local model is allowed to try at all: both sides
/// of one hunk and a little joining. A budget large enough for a whole file
/// would only buy a longer wrong answer.
const CONFLICT_HUNK_NUM_PREDICT: i32 = 1_024;

/// Stream tokens from the local mtplx chat endpoint, routing reasoning chunks
/// (and any inline `<think>...</think>` content) to [`GenMsg::Thinking`], and
/// content chunks to [`GenMsg::Output`].
/// Ends with a [`GenMsg::Done`] or [`GenMsg::Error`].
pub fn stream_commit_message(diff: String, tx: Sender<GenMsg>) {
    let settings = crate::settings::load();
    let limits = settings.clone();
    stream_prompt(
        build_commit_prompt(&diff, &settings),
        ChatTask::new("lg-commit", COMMIT_NUM_PREDICT, false),
        move |raw| crate::settings::enforce_commit_limits(&finalize(raw), &limits),
        tx,
    );
}

/// Derives this checkout's writing conventions — the language its commit
/// messages are written in, and candidate shapes for their format — from recent
/// history, and reports them on `tx` as a single
/// [`SettingsSuggestMsg`](crate::state::SettingsSuggestMsg).
pub fn suggest_repo_conventions(history: String, tx: Sender<crate::state::SettingsSuggestMsg>) {
    use crate::state::SettingsSuggestMsg;
    let (gen_tx, gen_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        stream_prompt(
            build_conventions_prompt(&history),
            ChatTask::new("lg-conventions", CONVENTIONS_NUM_PREDICT, false),
            |raw| strip_think_tags(raw).trim().to_string(),
            gen_tx,
        );
    });

    let mut output = String::new();
    let mut error = None;
    while let Ok(msg) = gen_rx.recv() {
        match msg {
            GenMsg::Output(chunk) => output.push_str(&chunk),
            GenMsg::Reset => output.clear(),
            // A shape cut off mid-line still parses as a shape, and would be
            // offered as the house style with nothing saying it is half of one.
            GenMsg::Done { stats, .. } if stats.truncated => {
                error = Some("conventions answer cut off at the token budget".to_string());
            }
            GenMsg::Done { text, .. } => output = text,
            GenMsg::Error(message) => error = Some(message),
            _ => {}
        }
    }
    let _ = tx.send(match error {
        Some(message) => SettingsSuggestMsg::Error(message),
        None => {
            let (language, shapes) = parse_conventions(&output);
            SettingsSuggestMsg::Done { language, shapes }
        }
    });
}

/// The overview goes in the system prompt, which stays the same for the whole walk and so is cached.
pub fn stream_guided_step(overview: String, file: String, tx: Sender<GenMsg>) {
    let messages = vec![
        ChatMessage {
            role: "system",
            content: build_guided_step_system_prompt(&overview, &crate::settings::load()),
        },
        ChatMessage {
            role: "user",
            content: file,
        },
    ];
    stream_messages(
        messages,
        ChatTask::new("lg-guided-step", GUIDED_STEP_NUM_PREDICT, false),
        finalize_review_chat,
        tx,
    );
}

/// What a guided review opens on: the plan its walk modes order the files by.
pub fn stream_guided_overview(context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_guided_overview_prompt(&context, &crate::settings::load()),
        ChatTask::new("lg-guided-overview", GUIDED_OVERVIEW_NUM_PREDICT, false),
        finalize_review_chat,
        tx,
    );
}

/// The whole change read across its files, after the walk through them.
pub fn stream_guided_final(context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_guided_final_prompt(&context, &crate::settings::load()),
        ChatTask::new("lg-guided-final", GUIDED_FINAL_NUM_PREDICT, false),
        finalize_review_chat,
        tx,
    );
}

/// A question about the step on screen, with what was already said about it.
pub fn stream_guided_question(
    context: String,
    commentary: String,
    question: String,
    tx: Sender<GenMsg>,
) {
    let mut messages = vec![ChatMessage {
        role: "system",
        content: build_guided_question_system_prompt(&context, &crate::settings::load()),
    }];
    if !commentary.trim().is_empty() {
        messages.push(ChatMessage {
            role: "user",
            content: "What do you make of this?".to_string(),
        });
        messages.push(ChatMessage {
            role: "assistant",
            content: commentary,
        });
    }
    messages.push(ChatMessage {
        role: "user",
        content: question,
    });
    stream_messages(
        messages,
        ChatTask::new("lg-guided-question", GUIDED_QUESTION_NUM_PREDICT, false),
        finalize_review_chat,
        tx,
    );
}

/// Ask for the merged lines that replace one conflict, and report them on `tx`
/// as an ordinary generation.
///
/// Reasoning is off: the answer is other people's code put back together, and
/// a model that deliberates about it spends its budget arguing rather than
/// merging. A conflict that needs the arguing is one to hand to claude instead.
/// Put one conflict, already written up by [`build_conflict_hunk_prompt`], to
/// the local model. The prompt is built by the caller so that what the model
/// was shown can be logged next to what it answered.
pub fn stream_conflict_hunk(prompt: String, tx: Sender<GenMsg>) {
    stream_prompt(
        prompt,
        ChatTask::new("lg-conflict", CONFLICT_HUNK_NUM_PREDICT, false),
        finalize_conflict_hunk,
        tx,
    );
}

pub(crate) use provider::{legacy_model, legacy_model_file};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_message_is_written_without_reasoning_first() {
        let task = ChatTask::new("lg-commit", COMMIT_NUM_PREDICT, false);

        assert!(!task.thinking);
    }

    /// A commit message that stops mid-word is committed as it stands, so
    /// nothing cuts one off part-way: the message is bounded by the limits the
    /// checkout configures, which apply to the finished message.
    #[test]
    fn a_commit_message_is_written_to_no_token_budget() {
        if std::env::var_os("LG_LLM_NUM_PREDICT").is_some() {
            return;
        }
        let task = ChatTask::new("lg-commit", COMMIT_NUM_PREDICT, false);

        assert_eq!(task.num_predict, 0, "the commit message has no cap");
    }
}
