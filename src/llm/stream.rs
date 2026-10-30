//! Posting a chat request and splitting the streamed response into messages.

use anyhow::Result;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::config::{LLM_NUM_PREDICT, LLM_TEMPERATURE, LLM_TOP_P};
use crate::state::GenMsg;

use super::provider::{
    LlmProvider, api_key, current_model, current_provider, endpoint_for_provider,
};
use super::stats::Tracked;
use consume::{consume_stream, trace_line};

mod consume;
mod usage;

#[derive(Serialize)]
struct ChatCompletionsRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    stream: bool,
    temperature: f32,
    top_p: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i32>,
    /// Off for the tasks that want an answer rather than deliberation: the
    /// server then spends no tokens on reasoning it would only be stripped
    /// back out of the reply.
    enable_thinking: bool,
    /// Names the server-side session, so consecutive requests for the same
    /// task reuse the prefill of their shared prompt prefix instead of
    /// re-reading it. Tasks are kept apart because their prefixes diverge.
    user: &'a str,
    /// Asks for the counters the server would otherwise keep to itself. Without
    /// this the stream ends at `[DONE]` with no token counts, no throughput,
    /// and — the one that matters — no way to tell an answer that finished from
    /// one that ran out of budget.
    stream_options: StreamOptions,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// The generation budget for a task that wants `budget` output tokens.
///
/// `LG_LLM_NUM_PREDICT` overrides `budget` while it is set; a value that does
/// not parse resolves to [`LLM_NUM_PREDICT`] rather than to `budget`.
pub fn num_predict_for(budget: i32) -> i32 {
    let Some(raw) = std::env::var_os("LG_LLM_NUM_PREDICT") else {
        return budget;
    };
    raw.to_str()
        .and_then(|value| value.parse().ok())
        .unwrap_or(LLM_NUM_PREDICT)
}

/// One task's generation settings: how much it may write, whether it should
/// reason first, and which server-side session its prefill belongs to.
#[derive(Clone, Copy)]
pub struct ChatTask {
    pub session: &'static str,
    pub num_predict: i32,
    pub thinking: bool,
}

impl ChatTask {
    /// A task wanting `budget` output tokens, honouring both the
    /// `LG_LLM_NUM_PREDICT` and `LG_LLM_THINKING` overrides.
    pub fn new(session: &'static str, budget: i32, thinking: bool) -> Self {
        Self {
            session,
            num_predict: num_predict_for(budget),
            thinking: thinking_override().unwrap_or(thinking),
        }
    }
}

/// `LG_LLM_THINKING` forces reasoning on or off for every task; anything that
/// does not read as a boolean leaves each task's own choice alone.
fn thinking_override() -> Option<bool> {
    match std::env::var("LG_LLM_THINKING")
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

pub fn stream_prompt<F>(prompt: String, task: ChatTask, finalizer: F, tx: Sender<GenMsg>)
where
    F: Fn(&str) -> String,
{
    stream_messages(
        vec![ChatMessage {
            role: "user",
            content: prompt,
        }],
        task,
        finalizer,
        tx,
    );
}

pub fn stream_messages(
    messages: Vec<ChatMessage>,
    task: ChatTask,
    finalizer: impl Fn(&str) -> String,
    tx: Sender<GenMsg>,
) {
    let provider = current_provider();
    if provider == LlmProvider::Claude {
        super::claude::stream_claude(messages, task, finalizer, tx);
        return;
    }
    let num_predict = task.num_predict;
    let start = Instant::now();
    let model = current_model();
    let endpoint = endpoint_for_provider(provider);
    let prompt_bytes = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();

    let body = match chat_request_body(&model, messages, task) {
        Ok(body) => body,
        Err(e) => {
            let _ = tx.send(GenMsg::Error(format!("llm request body: {e}")));
            return;
        }
    };

    let mut trace = std::env::var_os("LG_LLM_TRACE")
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());

    // Held for the whole request: dropping it is what takes the request back
    // out of the throughput readout, however this returns.
    let mut tracked = Tracked::new(prompt_bytes);

    trace_line(
        &mut trace,
        &format!(
            "# START provider={} model={model} endpoint={endpoint} num_predict={num_predict} thinking={} prompt_bytes={prompt_bytes} elapsed_ms=0",
            provider.label(),
            task.thinking,
        ),
    );

    let resp = match open_chat_stream(&endpoint, &body) {
        Ok(resp) => resp,
        Err(message) => {
            fail(&mut trace, &tx, message);
            return;
        }
    };

    consume_stream(
        BufReader::new(resp).lines(),
        finalizer,
        &tx,
        &mut trace,
        start,
        &mut tracked,
    );
}

/// POST the request and hand back the streaming response, or the message to
/// report the failure with.
fn open_chat_stream(
    endpoint: &str,
    body: &serde_json::Value,
) -> std::result::Result<reqwest::blocking::Response, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut request = client.post(endpoint).json(body);
    if let Some(key) = api_key() {
        request = request.bearer_auth(key);
    }
    request
        .send()
        .map_err(|e| format!("model server at {endpoint} {UNREACHABLE}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("model server at {endpoint} answered with an error: {e}"))
}

/// The word every "could not reach the server" error carries, so that a
/// consumer can tell it from a server that answered with a refusal.
pub(super) const UNREACHABLE: &str = "unreachable";

/// Whether a [`GenMsg::Error`] message means the server could not be reached
/// at all — a refused connection, a timeout — as opposed to a server that was
/// reached and turned the request down. The first says nothing else will get
/// through either; the second says nothing about the next request.
pub fn error_means_unreachable(message: &str) -> bool {
    message.contains(&format!(" {UNREACHABLE}: "))
}

/// Trace `message` and send it as the generation's [`GenMsg::Error`].
fn fail(trace: &mut Option<std::fs::File>, tx: &Sender<GenMsg>, message: String) {
    trace_line(trace, &format!("# ERROR {message}"));
    let _ = tx.send(GenMsg::Error(message));
}

fn chat_request_body(
    model: &str,
    messages: Vec<ChatMessage>,
    task: ChatTask,
) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(ChatCompletionsRequest {
        model,
        messages,
        stream: true,
        temperature: LLM_TEMPERATURE,
        top_p: LLM_TOP_P,
        max_tokens: (task.num_predict > 0).then_some(task.num_predict),
        enable_thinking: task.thinking,
        user: task.session,
        stream_options: StreamOptions {
            include_usage: true,
        },
    })?)
}

#[cfg(test)]
mod tests;
