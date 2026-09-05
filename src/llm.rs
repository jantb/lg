//! Asking the local model for a commit message, a review, or a PR description.

use std::sync::mpsc::Sender;

use crate::state::{GenMsg, ReviewChatMessage};

mod diff;
mod prompt;
mod provider;
mod reply;
mod stats;
mod stream;
mod think;

pub use prompt::GIVE_UP_PHRASE;
pub use provider::{
    LlmProvider, api_key, available_models, clear_saved_llm_settings, config_file_display,
    current_endpoint, current_model, current_provider, endpoint_for_provider, env_model_active,
    env_provider_active, prime_models_async, save_llm_settings,
};
pub use reply::parse_review_style_finding;

/// What an answer says when the server stopped it at the budget rather than at
/// the end.
///
/// An answer that was cut off reads exactly like one that finished, so anything
/// a person will act on says so in the text itself rather than only in a status
/// line that has since expired.
pub const TRUNCATED_NOTE: &str = "\u{2026} [cut off at the token budget]";
pub use stats::{GenStats, LlmPhase, forget_last_stats, last_stats, phase};
pub use stream::error_means_unreachable;

use prompt::{
    build_commit_prompt, build_conflict_hunk_prompt, build_conventions_prompt,
    build_review_assist_prompt, build_review_chat_system_prompt, build_review_pr_text_prompt,
    build_review_style_flag_prompt,
};
use reply::{
    finalize, finalize_conflict_hunk, finalize_review_assist, finalize_review_chat,
    finalize_review_pr_text, finalize_review_style_flag_for_path, parse_conventions,
};
use stream::{ChatMessage, ChatTask, stream_messages, stream_prompt};
use think::strip_think_tags;

/// Generation budget for a commit message. Reasoning is off for this task, so
/// the budget only has to cover the message itself.
const COMMIT_NUM_PREDICT: i32 = 512;
const CONVENTIONS_NUM_PREDICT: i32 = 300;
const REVIEW_ASSIST_NUM_PREDICT: i32 = 16_000;
/// The one task worth waiting for reasoning on: the whole point of the
/// assisted review is the model's analysis, not a formatted answer.
const REVIEW_ASSIST_THINKING: bool = true;
const REVIEW_PR_NUM_PREDICT: i32 = 4_096;
const REVIEW_CHAT_NUM_PREDICT: i32 = 768;
/// How many of the most recent chat turns travel with a follow-up question.
const REVIEW_CHAT_HISTORY_TURNS: usize = 8;
const REVIEW_STYLE_FLAG_NUM_PREDICT: i32 = 96;
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

pub fn stream_review_assist(context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_review_assist_prompt(&context, &crate::settings::load()),
        ChatTask::new(
            "lg-review-assist",
            REVIEW_ASSIST_NUM_PREDICT,
            REVIEW_ASSIST_THINKING,
        ),
        finalize_review_assist,
        tx,
    );
}

pub fn stream_review_style_flag(path: String, context: String, tx: Sender<GenMsg>) {
    let finalizer_path = path.clone();
    stream_prompt(
        build_review_style_flag_prompt(&path, &context, &crate::settings::load()),
        ChatTask::new("lg-review-flags", REVIEW_STYLE_FLAG_NUM_PREDICT, false),
        move |raw| finalize_review_style_flag_for_path(&finalizer_path, raw),
        tx,
    );
}

pub fn stream_review_pr_text(context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_review_pr_text_prompt(&context, &crate::settings::load()),
        ChatTask::new("lg-review-pr", REVIEW_PR_NUM_PREDICT, false),
        finalize_review_pr_text,
        tx,
    );
}

/// Ask for the merged lines that replace one conflict, and report them on `tx`
/// as an ordinary generation.
///
/// Reasoning is off: the answer is other people's code put back together, and
/// a model that deliberates about it spends its budget arguing rather than
/// merging. A conflict that needs the arguing is one to hand to claude instead.
pub fn stream_conflict_hunk(
    path: String,
    hunk: crate::git::ConflictHunk,
    before: String,
    after: String,
    tx: Sender<GenMsg>,
) {
    stream_prompt(
        build_conflict_hunk_prompt(&path, &hunk, &before, &after),
        ChatTask::new("lg-conflict", CONFLICT_HUNK_NUM_PREDICT, false),
        finalize_conflict_hunk,
        tx,
    );
}

pub fn stream_review_chat(
    context: String,
    history: Vec<ReviewChatMessage>,
    prompt: String,
    tx: Sender<GenMsg>,
) {
    let mut messages = vec![ChatMessage {
        role: "system",
        content: build_review_chat_system_prompt(&context, &crate::settings::load()),
    }];
    let window = history.len().saturating_sub(REVIEW_CHAT_HISTORY_TURNS);
    // A turn with nothing said — a failed request, shown in the chat as a
    // note — is not part of the conversation the model had.
    for message in history
        .into_iter()
        .skip(window)
        .filter(|message| !message.content.trim().is_empty())
    {
        messages.push(ChatMessage {
            role: message.role.as_chat_role(),
            content: message.content,
        });
    }
    messages.push(ChatMessage {
        role: "user",
        content: prompt,
    });
    stream_messages(
        messages,
        ChatTask::new("lg-review-chat", REVIEW_CHAT_NUM_PREDICT, false),
        finalize_review_chat,
        tx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LLM_NUM_PREDICT;

    #[test]
    fn a_commit_message_is_written_without_reasoning_first() {
        let task = ChatTask::new("lg-commit", COMMIT_NUM_PREDICT, false);

        assert!(!task.thinking);
        const { assert!(COMMIT_NUM_PREDICT > LLM_NUM_PREDICT) };
        if std::env::var_os("LG_LLM_NUM_PREDICT").is_none() {
            assert_eq!(task.num_predict, COMMIT_NUM_PREDICT);
        }
    }

    #[test]
    fn the_assisted_review_still_reasons_before_answering() {
        assert!(
            ChatTask::new(
                "lg-review-assist",
                REVIEW_ASSIST_NUM_PREDICT,
                REVIEW_ASSIST_THINKING
            )
            .thinking
        );
    }
}
