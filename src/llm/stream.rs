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
        .map_err(|e| format!("{} request: {e}", provider.label()))?
        .error_for_status()
        .map_err(|e| format!("{} status: {e}", provider.label()))
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
) {
    let mut output = StreamOutput::default();

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
            send_done(
                trace,
                &finalizer,
                &output,
                DoneStats::untimed("sse_done"),
                tx,
            );
            return;
        }
        let Some(json_line) = stream_json_line(&line) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json_line) else {
            continue;
        };

        if let Some(t) = stream_thinking_chunk(&v)
            && output.feed_thinking(t, tx).is_err()
        {
            return;
        }
        if let Some(c) = stream_output_chunk(&v)
            && output.feed(c, tx).is_err()
        {
            return;
        }
        if let Some(done) = stream_done_stats(&v) {
            output.flush(tx);
            send_done(trace, &finalizer, &output, done, tx);
            return;
        }
    }
    output.flush(tx);
    send_done(
        trace,
        &finalizer,
        &output,
        DoneStats::untimed("loop_exhausted"),
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

struct DoneStats {
    reason: String,
    eval_count: u64,
    prompt_eval_count: u64,
    total_ms: u64,
    eval_ms: u64,
}

impl DoneStats {
    /// Stats for an end the server sent no generation counters with.
    fn untimed(reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
            eval_count: 0,
            prompt_eval_count: 0,
            total_ms: 0,
            eval_ms: 0,
        }
    }
}

fn stream_done_stats(v: &serde_json::Value) -> Option<DoneStats> {
    (v.get("done").and_then(|done| done.as_bool()) == Some(true)).then(|| DoneStats {
        reason: v
            .get("done_reason")
            .and_then(|reason| reason.as_str())
            .unwrap_or("done")
            .to_string(),
        eval_count: json_u64(v, "eval_count"),
        prompt_eval_count: json_u64(v, "prompt_eval_count"),
        total_ms: nanos_to_ms(json_u64(v, "total_duration")),
        eval_ms: nanos_to_ms(json_u64(v, "eval_duration")),
    })
}

fn json_u64(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|value| value.as_u64()).unwrap_or(0)
}

fn nanos_to_ms(nanos: u64) -> u64 {
    nanos / 1_000_000
}

fn send_done(
    trace: &mut Option<std::fs::File>,
    finalizer: &dyn Fn(&str) -> String,
    output: &StreamOutput,
    stats: DoneStats,
    tx: &Sender<GenMsg>,
) {
    let final_output = finalizer(&output.text);
    if let Some(f) = trace.as_mut() {
        let _ = writeln!(
            f,
            "# DONE done_reason={} eval_count={} prompt_eval_count={} total_duration_ms={} eval_duration_ms={} think_bytes={} out_bytes={} final_output={final_output:?}",
            stats.reason,
            stats.eval_count,
            stats.prompt_eval_count,
            stats.total_ms,
            stats.eval_ms,
            output.think_bytes,
            output.out_bytes,
        );
    }
    let _ = tx.send(GenMsg::Done(final_output));
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
        );
        drop(tx);
        rx.iter().collect()
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
        assert!(matches!(&msgs[2], GenMsg::Done(s) if s == "feat: add a thing"));
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn a_stream_that_never_reports_done_still_finishes() {
        let msgs = drive(vec![Ok(
            r#"{"message":{"content":"only this"}}"#.to_string()
        )]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "only this"));
        assert!(matches!(&msgs[1], GenMsg::Done(s) if s == "only this"));
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
        assert!(matches!(&msgs[2], GenMsg::Done(s) if s == "answer<thi"));
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
        assert!(matches!(&msgs[1], GenMsg::Done(s) if s == "kept"));
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
        assert!(matches!(&msgs[3], GenMsg::Done(s) if s == "answer done"));
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
        assert!(matches!(&msgs[2], GenMsg::Done(s) if s == "the answer"));
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
        assert!(matches!(&msgs[1], GenMsg::Done(s) if s == "survived"));
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

    #[test]
    fn a_bare_close_mid_stream_resets_what_was_sent_as_output() {
        let msgs = drive(vec![
            Ok(r#"{"message":{"content":"draft answer"}}"#.to_string()),
            Ok(r#"{"message":{"content":"</think>real answer"}}"#.to_string()),
            Ok(r#"{"done":true}"#.to_string()),
        ]);

        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "draft answer"));
        assert!(matches!(&msgs[1], GenMsg::Reset));
        assert!(matches!(&msgs[3], GenMsg::Done(s) if s == "real answer"));
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
        let value: serde_json::Value = serde_json::from_str(
            r#"{"done":true,"done_reason":"stop","eval_count":9,"prompt_eval_count":7,"total_duration":3000000,"eval_duration":2000000}"#,
        )
        .unwrap();
        let stats = stream_done_stats(&value).unwrap();

        assert_eq!(stats.reason, "stop");
        assert_eq!(stats.eval_count, 9);
        assert_eq!(stats.prompt_eval_count, 7);
        assert_eq!(stats.total_ms, 3);
        assert_eq!(stats.eval_ms, 2);
    }
}
