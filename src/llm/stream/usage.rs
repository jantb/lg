//! What the server said an answer cost: its token counts and rates, and the
//! model that served it.

use crate::llm::stats::GenStats;

/// The counters an OpenAI-shaped stream reports on its final usage chunk. Only
/// sent when the request asked for them, which is why every request does.
pub(super) fn stream_usage_stats(v: &serde_json::Value) -> Option<GenStats> {
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
pub(super) fn stream_done_stats(v: &serde_json::Value) -> Option<GenStats> {
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

pub(super) fn served_model(v: &serde_json::Value) -> Option<String> {
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
pub(super) fn rate(tokens: u64, nanos: u64) -> f64 {
    if nanos == 0 {
        return 0.0;
    }
    tokens as f64 * 1_000_000_000.0 / nanos as f64
}
