//! Asking the local model for a commit message, a review, or a PR description.

use std::sync::mpsc::Sender;

use crate::state::{GenMsg, ReviewChatMessage};

mod diff;
mod prompt;
mod provider;
mod reply;
mod stream;
mod think;

pub use provider::{
    LlmProvider, clear_saved_llm_settings, config_file_display, current_endpoint, current_model,
    current_provider, endpoint_for_provider, env_model_active, env_provider_active,
    save_llm_settings,
};
pub use reply::parse_review_style_finding;

use prompt::{
    build_commit_prompt, build_conventions_prompt, build_review_assist_prompt,
    build_review_chat_system_prompt, build_review_pr_text_prompt, build_review_style_flag_prompt,
};
use reply::{
    finalize, finalize_review_assist, finalize_review_chat, finalize_review_pr_text,
    finalize_review_style_flag_for_path, parse_conventions,
};
use stream::{ChatMessage, num_predict_for, stream_messages, stream_prompt};
use think::strip_think_tags;

/// Generation budget for a commit message. The model streams its reasoning
/// into the same budget as its answer, so this has to cover both.
const COMMIT_NUM_PREDICT: i32 = 8_192;
const CONVENTIONS_NUM_PREDICT: i32 = 300;
const REVIEW_ASSIST_NUM_PREDICT: i32 = 16_000;
const REVIEW_PR_NUM_PREDICT: i32 = 4_096;
const REVIEW_CHAT_NUM_PREDICT: i32 = 768;
/// How many of the most recent chat turns travel with a follow-up question.
const REVIEW_CHAT_HISTORY_TURNS: usize = 8;
const REVIEW_STYLE_FLAG_NUM_PREDICT: i32 = 96;

/// Stream tokens from the local Ollama chat endpoint, routing reasoning chunks
/// (and any inline `<think>...</think>` content) to [`GenMsg::Thinking`], and
/// content chunks to [`GenMsg::Output`].
/// Ends with a [`GenMsg::Done`] or [`GenMsg::Error`].
pub fn stream_commit_message(diff: String, tx: Sender<GenMsg>) {
    let settings = crate::settings::load();
    let limits = settings.clone();
    stream_prompt(
        build_commit_prompt(&diff, &settings),
        num_predict_for(COMMIT_NUM_PREDICT),
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
            num_predict_for(CONVENTIONS_NUM_PREDICT),
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
            GenMsg::Done(text) => output = text,
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
        num_predict_for(REVIEW_ASSIST_NUM_PREDICT),
        finalize_review_assist,
        tx,
    );
}

pub fn stream_review_style_flag(path: String, context: String, tx: Sender<GenMsg>) {
    let finalizer_path = path.clone();
    stream_prompt(
        build_review_style_flag_prompt(&path, &context, &crate::settings::load()),
        num_predict_for(REVIEW_STYLE_FLAG_NUM_PREDICT),
        move |raw| finalize_review_style_flag_for_path(&finalizer_path, raw),
        tx,
    );
}

pub fn stream_review_pr_text(context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_review_pr_text_prompt(&context, &crate::settings::load()),
        num_predict_for(REVIEW_PR_NUM_PREDICT),
        finalize_review_pr_text,
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
    for message in history.into_iter().skip(window) {
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
        num_predict_for(REVIEW_CHAT_NUM_PREDICT),
        finalize_review_chat,
        tx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LLM_NUM_PREDICT;

    #[test]
    fn commit_options_leave_room_for_reasoning() {
        assert_eq!(COMMIT_NUM_PREDICT, 8_192);
        const { assert!(COMMIT_NUM_PREDICT > LLM_NUM_PREDICT) };
        if std::env::var_os("LG_LLM_NUM_PREDICT").is_none() {
            assert_eq!(num_predict_for(COMMIT_NUM_PREDICT), COMMIT_NUM_PREDICT);
        }
    }
}
