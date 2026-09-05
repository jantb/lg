//! Posting a chat request and splitting the streamed response into messages.

use anyhow::Result;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::config::{LLM_NUM_PREDICT, LLM_TEMPERATURE, LLM_TOP_P};
use crate::state::GenMsg;

use super::provider::{
    LlmProvider, api_key, current_model, current_provider, endpoint_for_provider,
};
use super::stats::{GenStats, Tracked};
use super::think::ThinkSplit;

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
    let num_predict = task.num_predict;
    let start = Instant::now();
    let model = current_model();
    let provider = current_provider();
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

    let resp = match open_chat_stream(&endpoint, &body, provider) {
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
    provider: LlmProvider,
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
        .map_err(|e| format!("{} {UNREACHABLE}: {e}", provider.label()))?
        .error_for_status()
        .map_err(|e| format!("{} status: {e}", provider.label()))
}

/// The word every "could not reach the server" error carries, so that a
/// consumer can tell it from a server that answered with a refusal.
const UNREACHABLE: &str = "unreachable";

/// Whether a [`GenMsg::Error`] message means the server could not be reached
/// at all — a refused connection, a timeout — as opposed to a server that was
/// reached and turned the request down. The first says nothing else will get
/// through either; the second says nothing about the next request.
pub fn error_means_unreachable(message: &str) -> bool {
    message.contains(&format!(" {UNREACHABLE}: "))
}

/// The output accumulated from one response stream, and the byte counts the
/// trace log reports alongside it.
#[derive(Default)]
struct StreamOutput {
    parser: ThinkSplit,
    text: String,
    think_bytes: usize,
    out_bytes: usize,
}

impl StreamOutput {
    /// Route a content chunk, splitting any `<think>` span out of it.
    /// `Err(())` means the receiver is gone.
    fn feed(&mut self, chunk: &str, tx: &Sender<GenMsg>) -> std::result::Result<(), ()> {
        let (think, out) = self.parser.feed(chunk, tx, &mut self.text)?;
        self.think_bytes += think;
        self.out_bytes += out;
        Ok(())
    }

    /// Route a chunk the server itself labelled as reasoning.
    fn feed_thinking(&mut self, chunk: &str, tx: &Sender<GenMsg>) -> std::result::Result<(), ()> {
        self.think_bytes += chunk.len();
        tx.send(GenMsg::Thinking(chunk.to_owned())).map_err(|_| ())
    }

    /// Release whatever the tag splitter is still holding back.
    fn flush(&mut self, tx: &Sender<GenMsg>) {
        let (think, out) = self.parser.flush(tx, &mut self.text).unwrap_or((0, 0));
        self.think_bytes += think;
        self.out_bytes += out;
    }
}

/// Split the response body's lines between [`GenMsg::Thinking`] and
/// [`GenMsg::Output`], ending with exactly one [`GenMsg::Done`] or
/// [`GenMsg::Error`].
///
/// Both an NDJSON `done` object and an SSE `[DONE]` marker end the stream;
/// so does the iterator running out. A line that is blank, is not JSON, or
/// carries neither a chunk nor a done marker is skipped.
fn consume_stream(
    lines: impl Iterator<Item = std::io::Result<String>>,
    finalizer: impl Fn(&str) -> String,
    tx: &Sender<GenMsg>,
    trace: &mut Option<std::fs::File>,
    start: Instant,
    tracked: &mut Tracked,
) {
    let mut output = StreamOutput::default();
    let mut end = StreamEnd::default();

    for line in lines {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                trace_event(trace, start, &output, &format!("# ERROR stream read: {e}"));
                fail(trace, tx, format!("stream read: {e}"));
                return;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        trace_event(trace, start, &output, &line);
        if stream_sse_done_line(&line) {
            output.flush(tx);
            send_done(trace, &finalizer, &output, end, "sse_done", tracked, tx);
            return;
        }
        let Some(json_line) = stream_json_line(&line) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json_line) else {
            continue;
        };
        // A server that has already sent its 200 can only refuse from inside
        // the stream. Such a chunk carries no answer, and treating it as the
        // end of one would report an empty message as generated.
        if let Some(message) = stream_error(&v) {
            fail(trace, tx, format!("llm server: {message}"));
            return;
        }
        end.absorb(&v);

        if let Some(t) = stream_thinking_chunk(&v) {
            tracked.note_token();
            if output.feed_thinking(t, tx).is_err() {
                return;
            }
        }
        if let Some(c) = stream_output_chunk(&v) {
            tracked.note_token();
            if output.feed(c, tx).is_err() {
                return;
            }
        }
        // An Ollama-shaped stream ends on its own object rather than on a
        // marker line, and nothing follows it.
        if v.get("done").and_then(|done| done.as_bool()) == Some(true) {
            output.flush(tx);
            send_done(trace, &finalizer, &output, end, "done", tracked, tx);
            return;
        }
    }
    output.flush(tx);
    send_done(
        trace,
        &finalizer,
        &output,
        end,
        "loop_exhausted",
        tracked,
        tx,
    );
}

/// Append one line to the trace log, if one is open.
fn trace_line(trace: &mut Option<std::fs::File>, line: &str) {
    if let Some(f) = trace.as_mut() {
        let _ = writeln!(f, "{line}");
    }
}

/// Append one line stamped with the elapsed time and the byte counts so far.
fn trace_event(
    trace: &mut Option<std::fs::File>,
    start: Instant,
    output: &StreamOutput,
    payload: &str,
) {
    trace_line(
        trace,
        &format!(
            "+T{} think_bytes={} out_bytes={} | {payload}",
            start.elapsed().as_millis(),
            output.think_bytes,
            output.out_bytes,
        ),
    );
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

fn stream_sse_done_line(line: &str) -> bool {
    line.trim() == "data: [DONE]"
}

fn stream_json_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if let Some(data) = trimmed.strip_prefix("data:") {
        let data = data.trim();
        (!data.is_empty() && data != "[DONE]").then_some(data)
    } else {
        trimmed.starts_with('{').then_some(trimmed)
    }
}

/// The message of an error the server reported inside the stream, in either
/// the OpenAI shape (an `error` object, with `finish_reason: "error"` on the
/// choice) or the Ollama shape (a bare `error` string).
fn stream_error(v: &serde_json::Value) -> Option<String> {
    let error = v.get("error").filter(|error| !error.is_null())?;
    let message = error
        .as_str()
        .or_else(|| error.get("message").and_then(|message| message.as_str()))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_owned);
    Some(message.unwrap_or_else(|| {
        stream_finish_reason(v)
            .map_or_else(|| "request failed".to_string(), |reason| reason.to_string())
    }))
}

fn stream_thinking_chunk(v: &serde_json::Value) -> Option<&str> {
    v.pointer("/choices/0/delta/reasoning_content")
        .or_else(|| v.pointer("/choices/0/delta/thinking"))
        .or_else(|| v.pointer("/message/thinking"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

fn stream_output_chunk(v: &serde_json::Value) -> Option<&str> {
    v.pointer("/choices/0/delta/content")
        .or_else(|| v.pointer("/choices/0/message/content"))
        .or_else(|| v.pointer("/message/content"))
        .or_else(|| v.pointer("/response"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

/// The counters a response carries, wherever in the stream they turn up.
///
/// An OpenAI-shaped stream reports the reason it stopped on the last content
/// chunk and its token counts on a separate usage chunk after it; an
/// Ollama-shaped one puts both in one `done` object. Either way what arrived is
/// folded in here, and whatever was collected by the time the stream ends is
/// what gets reported.
#[derive(Default)]
struct StreamEnd {
    reason: Option<String>,
    stats: Option<GenStats>,
}

impl StreamEnd {
    fn absorb(&mut self, v: &serde_json::Value) {
        if let Some(reason) = stream_finish_reason(v) {
            self.reason = Some(reason.to_string());
        }
        if let Some(stats) = stream_usage_stats(v).or_else(|| stream_done_stats(v)) {
            self.stats = Some(stats);
        }
    }

    /// The reason the stream ended and the stats to report with it.
    ///
    /// `truncated` is settled here rather than at either parse site, because
    /// the reason and the counters can arrive on different chunks and neither
    /// one alone knows the answer.
    fn resolve(self, fallback_reason: &str) -> (String, GenStats) {
        let reason = self.reason.unwrap_or_else(|| fallback_reason.to_string());
        let mut stats = self.stats.unwrap_or_default();
        stats.truncated = reason == "length";
        (reason, stats)
    }
}

fn stream_finish_reason(v: &serde_json::Value) -> Option<&str> {
    v.pointer("/choices/0/finish_reason")
        .or_else(|| v.get("done_reason"))
        .and_then(|reason| reason.as_str())
        .filter(|reason| !reason.is_empty())
}

/// The counters an OpenAI-shaped stream reports on its final usage chunk. Only
/// sent when the request asked for them, which is why every request does.
fn stream_usage_stats(v: &serde_json::Value) -> Option<GenStats> {
    let usage = v.get("usage").filter(|usage| !usage.is_null())?;
    let timings = v.get("timings");
    // mtplx reports the same two rates twice, under its own names as well as
    // the llama.cpp ones. Either will do; taking both means neither spelling
    // has to be the one the server happens to use.
    let server = v.get("mtplx_stats");
    Some(GenStats {
        prompt_tokens: json_u64(usage, "prompt_tokens"),
        cached_tokens: usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        completion_tokens: json_u64(usage, "completion_tokens"),
        prefill_tps: json_rate(timings, "prompt_per_second")
            .or_else(|| json_rate(server, "prefill_tok_s"))
            .unwrap_or(0.0),
        decode_tps: json_rate(timings, "predicted_per_second")
            .or_else(|| json_rate(server, "decode_tok_s"))
            .unwrap_or(0.0),
        truncated: false,
        served_model: served_model(v),
    })
}

/// The counters an Ollama-shaped stream reports in its `done` object.
fn stream_done_stats(v: &serde_json::Value) -> Option<GenStats> {
    (v.get("done").and_then(|done| done.as_bool()) == Some(true)).then(|| {
        let prompt_tokens = json_u64(v, "prompt_eval_count");
        let completion_tokens = json_u64(v, "eval_count");
        GenStats {
            prompt_tokens,
            cached_tokens: 0,
            completion_tokens,
            prefill_tps: rate(prompt_tokens, json_u64(v, "prompt_eval_duration")),
            decode_tps: rate(completion_tokens, json_u64(v, "eval_duration")),
            truncated: false,
            served_model: served_model(v),
        }
    })
}

fn served_model(v: &serde_json::Value) -> Option<String> {
    v.get("model")
        .and_then(|model| model.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

fn json_u64(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|value| value.as_u64()).unwrap_or(0)
}

/// A rate the server stated, ignoring anything that is not a usable number.
fn json_rate(v: Option<&serde_json::Value>, key: &str) -> Option<f64> {
    v?.get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|rate| rate.is_finite() && *rate > 0.0)
}

/// Tokens per second over a span given in nanoseconds.
fn rate(tokens: u64, nanos: u64) -> f64 {
    if nanos == 0 {
        return 0.0;
    }
    tokens as f64 * 1_000_000_000.0 / nanos as f64
}

/// Report the end of a stream: the stats first, so a consumer can judge the
/// answer before it takes delivery of it, and then the answer itself.
fn send_done(
    trace: &mut Option<std::fs::File>,
    finalizer: &dyn Fn(&str) -> String,
    output: &StreamOutput,
    end: StreamEnd,
    fallback_reason: &str,
    tracked: &mut Tracked,
    tx: &Sender<GenMsg>,
) {
    let final_output = finalizer(&output.text);
    let (reason, stats) = end.resolve(fallback_reason);
    if let Some(f) = trace.as_mut() {
        let _ = writeln!(
            f,
            "# DONE finish_reason={reason} truncated={} prompt_tokens={} cached_tokens={} completion_tokens={} prefill_tps={:.1} decode_tps={:.1} served_model={:?} think_bytes={} out_bytes={} final_output={final_output:?}",
            stats.truncated,
            stats.prompt_tokens,
            stats.cached_tokens,
            stats.completion_tokens,
            stats.prefill_tps,
            stats.decode_tps,
            stats.served_model,
            output.think_bytes,
            output.out_bytes,
        );
    }
    tracked.report(stats.clone());
    let _ = tx.send(GenMsg::Done {
        text: final_output,
        stats,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// Drive [`consume_stream`] over `lines` with an identity finalizer and no
    /// trace, and collect everything it sent.
    fn drive(lines: Vec<std::io::Result<String>>) -> Vec<GenMsg> {
        let (tx, rx) = channel::<GenMsg>();
        consume_stream(
            lines.into_iter(),
            |raw: &str| raw.to_string(),
            &tx,
            &mut None,
            Instant::now(),
            &mut Tracked::new(0),
        );
        drop(tx);
        rx.iter().collect()
    }

    fn drive_stats(lines: Vec<std::io::Result<String>>) -> GenStats {
        drive(lines)
            .into_iter()
            .find_map(|msg| match msg {
                GenMsg::Done { stats, .. } => Some(stats),
                _ => None,
            })
            .expect("a stream reports what it cost")
    }

    #[test]
    fn a_stream_routes_ndjson_content_and_ends_on_the_done_object() {
        let msgs = drive(vec![
            Ok(r#"{"message":{"content":"feat: "}}"#.to_string()),
            Ok(r#"{"message":{"content":"add a thing"}}"#.to_string()),
            Ok(r#"{"done":true,"done_reason":"stop"}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "feat: "));
        assert!(matches!(&msgs[1], GenMsg::Output(s) if s == "add a thing"));
        assert!(matches!(&msgs[2], GenMsg::Done { text: s, .. } if s == "feat: add a thing"));
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn a_stream_that_never_reports_done_still_finishes() {
        let msgs = drive(vec![Ok(
            r#"{"message":{"content":"only this"}}"#.to_string()
        )]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "only this"));
        assert!(matches!(&msgs[1], GenMsg::Done { text: s, .. } if s == "only this"));
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn a_stream_ending_mid_tag_still_releases_the_held_bytes() {
        // The parser holds back a partial `<think>` prefix, so only the flush
        // after the last line can send it.
        let msgs = drive(vec![Ok(
            r#"{"message":{"content":"answer<thi"}}"#.to_string()
        )]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "answer"));
        assert!(matches!(&msgs[1], GenMsg::Output(s) if s == "<thi"));
        assert!(matches!(&msgs[2], GenMsg::Done { text: s, .. } if s == "answer<thi"));
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn a_stream_stops_at_the_sse_done_marker() {
        let msgs = drive(vec![
            Ok(r#"data: {"choices":[{"delta":{"content":"kept"}}]}"#.to_string()),
            Ok("data: [DONE]".to_string()),
            Ok(r#"data: {"choices":[{"delta":{"content":"ignored"}}]}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "kept"));
        assert!(matches!(&msgs[1], GenMsg::Done { text: s, .. } if s == "kept"));
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn a_stream_splits_think_tags_spanning_chunks() {
        let msgs = drive(vec![
            Ok(r#"{"message":{"content":"answer<thi"}}"#.to_string()),
            Ok(r#"{"message":{"content":"nk>hmm</think> done"}}"#.to_string()),
            Ok(r#"{"done":true}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "answer"));
        assert!(matches!(&msgs[1], GenMsg::Thinking(s) if s == "hmm"));
        assert!(matches!(&msgs[2], GenMsg::Output(s) if s == " done"));
        assert!(matches!(&msgs[3], GenMsg::Done { text: s, .. } if s == "answer done"));
    }

    #[test]
    fn a_stream_sends_a_reasoning_field_as_thinking() {
        let msgs = drive(vec![
            Ok(r#"{"choices":[{"delta":{"reasoning_content":"weighing it"}}]}"#.to_string()),
            Ok(r#"{"message":{"content":"the answer"}}"#.to_string()),
            Ok(r#"{"done":true}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Thinking(s) if s == "weighing it"));
        assert!(matches!(&msgs[1], GenMsg::Output(s) if s == "the answer"));
        assert!(matches!(&msgs[2], GenMsg::Done { text: s, .. } if s == "the answer"));
    }

    #[test]
    fn a_stream_skips_blank_and_unparsable_lines() {
        let msgs = drive(vec![
            Ok(String::new()),
            Ok("   ".to_string()),
            Ok("not json at all".to_string()),
            Ok("{ broken".to_string()),
            Ok(r#"{"unrelated":"field"}"#.to_string()),
            Ok(r#"{"message":{"content":"survived"}}"#.to_string()),
            Ok(r#"{"done":true}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "survived"));
        assert!(matches!(&msgs[1], GenMsg::Done { text: s, .. } if s == "survived"));
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn a_read_error_ends_the_stream_with_an_error() {
        let msgs = drive(vec![
            Ok(r#"{"message":{"content":"partial"}}"#.to_string()),
            Err(std::io::Error::other("socket closed")),
            Ok(r#"{"done":true}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "partial"));
        assert!(matches!(&msgs[1], GenMsg::Error(s) if s == "stream read: socket closed"));
        assert_eq!(msgs.len(), 2);
    }

    /// mtplx refuses a request whose session is already busy from inside an
    /// otherwise successful stream: one chunk, no content, an `error` object.
    /// Reading that as the end of an answer left an empty commit message on
    /// screen under a status saying it had been generated.
    #[test]
    fn an_error_chunk_ends_the_stream_as_an_error() {
        let msgs = drive(vec![Ok(
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"error"}],"error":{"message":"session lg-commit is already in flight","type":"conflict_error"}}"#.to_string(),
        )]);

        assert_eq!(msgs.len(), 1);
        assert!(
            matches!(&msgs[0], GenMsg::Error(s) if s.contains("session lg-commit is already in flight")),
            "{msgs:?}"
        );
    }

    #[test]
    fn only_a_connection_failure_reads_as_unreachable() {
        assert!(error_means_unreachable(&format!(
            "mtplx {UNREACHABLE}: error sending request"
        )));
        assert!(!error_means_unreachable("mtplx status: HTTP 413"));
        assert!(!error_means_unreachable(
            "llm server: session lg-conflict is already in flight"
        ));
    }

    #[test]
    fn a_bare_error_string_is_reported_too() {
        let msgs = drive(vec![Ok(r#"{"error":"model not found"}"#.to_string())]);

        assert!(matches!(&msgs[0], GenMsg::Error(s) if s.contains("model not found")));
    }

    #[test]
    fn a_bare_close_mid_stream_resets_what_was_sent_as_output() {
        let msgs = drive(vec![
            Ok(r#"{"message":{"content":"draft answer"}}"#.to_string()),
            Ok(r#"{"message":{"content":"</think>real answer"}}"#.to_string()),
            Ok(r#"{"done":true}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "draft answer"));
        assert!(matches!(&msgs[1], GenMsg::Reset));
        assert!(matches!(&msgs[3], GenMsg::Done { text: s, .. } if s == "real answer"));
    }

    #[test]
    fn chat_request_uses_the_openai_completions_shape() {
        let body = chat_request_body(
            "qwen-local",
            vec![ChatMessage {
                role: "user",
                content: "hi".into(),
            }],
            ChatTask {
                session: "lg-test",
                num_predict: 42,
                thinking: false,
            },
        )
        .unwrap();

        assert_eq!(body["model"], "qwen-local");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 42);
        assert_eq!(body["enable_thinking"], false);
        assert_eq!(body["user"], "lg-test");
        assert_eq!(body["temperature"], LLM_TEMPERATURE);
        assert_eq!(body["top_p"], LLM_TOP_P);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn a_reader_still_accepts_ndjson_chunks() {
        let line = r#"{"message":{"content":"hello","thinking":"plan"},"done":false}"#;
        let json = stream_json_line(line).unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();

        assert_eq!(stream_output_chunk(&value), Some("hello"));
        assert_eq!(stream_thinking_chunk(&value), Some("plan"));
        assert!(stream_done_stats(&value).is_none());
    }

    #[test]
    fn mtplx_sse_deltas_are_read_as_output() {
        let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let json = stream_json_line(line).unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();

        assert_eq!(stream_output_chunk(&value), Some("hello"));
        assert!(stream_sse_done_line("data: [DONE]"));
    }

    #[test]
    fn a_done_object_reads_generation_stats() {
        let stats = drive_stats(vec![Ok(r#"{"done":true,"done_reason":"stop","model":"local","eval_count":10,"prompt_eval_count":8,"eval_duration":2000000000,"prompt_eval_duration":1000000000}"#.to_string())]);

        assert_eq!(stats.prompt_tokens, 8);
        assert_eq!(stats.completion_tokens, 10);
        assert_eq!(stats.prefill_tps, 8.0);
        assert_eq!(stats.decode_tps, 5.0);
        assert_eq!(stats.served_model.as_deref(), Some("local"));
        assert!(!stats.truncated);
    }

    /// The shape mtplx actually sends: the counters ride a final usage chunk
    /// after the last content, and the stream then ends on the marker line.
    /// Reading only an Ollama-style `done` object left every one of these at
    /// zero.
    #[test]
    fn a_usage_chunk_reports_prefill_and_decode_throughput() {
        let stats = drive_stats(vec![
            Ok(r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#.to_string()),
            Ok(
                r#"data: {"model":"mtplx-qwen","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1417,"completion_tokens":412,"prompt_tokens_details":{"cached_tokens":1280}},"timings":{"prompt_per_second":125.0,"predicted_per_second":16.9}}"#
                    .to_string(),
            ),
            Ok("data: [DONE]".to_string()),
        ]);

        assert_eq!(stats.prompt_tokens, 1417);
        assert_eq!(stats.cached_tokens, 1280);
        assert_eq!(stats.completion_tokens, 412);
        assert_eq!(stats.prefill_tps, 125.0);
        assert_eq!(stats.decode_tps, 16.9);
        assert_eq!(stats.served_model.as_deref(), Some("mtplx-qwen"));
        assert!(!stats.truncated);
    }

    /// The one stat a caller acts on rather than draws. An answer that stopped
    /// because it ran out of budget is not an answer, and only the server can
    /// say which of the two it was.
    #[test]
    fn an_answer_cut_off_at_the_budget_is_reported_as_truncated() {
        let stats = drive_stats(vec![
            Ok(r#"data: {"choices":[{"delta":{"content":"half a mer"}}]}"#.to_string()),
            Ok(
                r#"data: {"choices":[{"delta":{},"finish_reason":"length"}],"usage":{"prompt_tokens":40,"completion_tokens":1024}}"#
                    .to_string(),
            ),
            Ok("data: [DONE]".to_string()),
        ]);

        assert!(stats.truncated);
    }

    /// mtplx spells the same two rates under its own names as well; a server
    /// that sends only those is read just the same.
    #[test]
    fn throughput_is_read_from_the_server_specific_names_too() {
        let stats = drive_stats(vec![
            Ok(
                r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":4},"mtplx_stats":{"prefill_tok_s":48.9,"decode_tok_s":25.5}}"#
                    .to_string(),
            ),
            Ok("data: [DONE]".to_string()),
        ]);

        assert_eq!(stats.prefill_tps, 48.9);
        assert_eq!(stats.decode_tps, 25.5);
    }

    #[test]
    fn a_stream_that_reports_nothing_still_ends_with_stats() {
        let stats = drive_stats(vec![Ok(
            r#"{"message":{"content":"only this"}}"#.to_string()
        )]);

        assert!(!stats.is_measured(), "there was nothing to measure");
        assert!(!stats.truncated, "silence is not a truncation");
    }

    #[test]
    fn every_request_asks_for_the_counters_it_needs() {
        let body = chat_request_body(
            "qwen-local",
            vec![ChatMessage {
                role: "user",
                content: "hi".into(),
            }],
            ChatTask {
                session: "lg-test",
                num_predict: 42,
                thinking: false,
            },
        )
        .unwrap();

        assert_eq!(
            body["stream_options"]["include_usage"], true,
            "without this the server sends no usage chunk at all"
        );
    }
}
