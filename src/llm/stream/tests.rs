use std::sync::mpsc::channel;

use crate::llm::stats::GenStats;

use super::consume::{
    consume_stream, stream_json_line, stream_output_chunk, stream_sse_done_line,
    stream_thinking_chunk,
};
use super::usage::stream_done_stats;
use super::*;

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

/// The message names the endpoint it could not reach, in words a user
/// can act on, and still reads as unreachable to the code that stops
/// asking.
#[test]
fn a_refused_connection_names_the_endpoint() {
    let endpoint = "http://127.0.0.1:1/v1/chat/completions";
    let Err(message) = open_chat_stream(endpoint, &serde_json::json!({})) else {
        panic!("nothing listens on port 1");
    };
    assert!(error_means_unreachable(&message), "{message}");
    assert!(
        message.starts_with(&format!("model server at {endpoint} unreachable")),
        "{message}"
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

/// Zero is lg saying "no cap", not "write nothing": the field has to be
/// left off the request rather than sent as a zero the server would honour.
#[test]
fn a_task_without_a_budget_asks_for_no_cap() {
    let body = chat_request_body(
        "qwen-local",
        vec![ChatMessage {
            role: "user",
            content: "hi".into(),
        }],
        ChatTask {
            session: "lg-test",
            num_predict: 0,
            thinking: false,
        },
    )
    .unwrap();

    assert!(
        body.get("max_tokens").is_none(),
        "a budgetless task must not send a cap: {body}"
    );
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
