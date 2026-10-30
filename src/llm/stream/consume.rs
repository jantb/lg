//! Reading a streamed answer: each event of the stream, the thinking and the
//! answer told apart, and how it ended.

use std::io::Write;
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::llm::stats::{GenStats, Tracked};
use crate::llm::think::ThinkSplit;
use crate::state::GenMsg;

use super::fail;
use super::usage::{stream_done_stats, stream_usage_stats};

/// The output accumulated from one response stream, and the byte counts the
/// trace log reports alongside it.
#[derive(Default)]
pub(super) struct StreamOutput {
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
pub(super) fn consume_stream(
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

        if let Some(tokens) = stream_progress_tokens(&v) {
            tracked.note_progress(tokens);
        }
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
pub(super) fn trace_line(trace: &mut Option<std::fs::File>, line: &str) {
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

pub(super) fn stream_sse_done_line(line: &str) -> bool {
    line.trim() == "data: [DONE]"
}

pub(super) fn stream_json_line(line: &str) -> Option<&str> {
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

/// How many tokens the server has committed so far, from the progress chunks
/// mtplx interleaves with the content. A server that speculates commits
/// several tokens per chunk, so this is the count the live decode rate wants
/// and the chunk count is only what stands in for it.
fn stream_progress_tokens(v: &serde_json::Value) -> Option<u64> {
    v.pointer("/mtplx_progress/completion_tokens")
        .and_then(serde_json::Value::as_u64)
}

pub(super) fn stream_thinking_chunk(v: &serde_json::Value) -> Option<&str> {
    v.pointer("/choices/0/delta/reasoning_content")
        .or_else(|| v.pointer("/choices/0/delta/thinking"))
        .or_else(|| v.pointer("/message/thinking"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

pub(super) fn stream_output_chunk(v: &serde_json::Value) -> Option<&str> {
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
pub(super) struct StreamEnd {
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
