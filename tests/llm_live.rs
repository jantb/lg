//! Checks against a real model server, ignored by default.
//!
//! The stream parser has unit tests over recorded lines, which is what catches
//! a mistake in the parsing. What they cannot catch is the server changing its
//! mind about what it sends — and the counters lg needs are only sent because
//! the request asks for them, so "the server stopped answering that way" looks
//! exactly like "the feature was never wired up".
//!
//! Run with a server up:
//!
//! ```text
//! cargo test --test llm_live -- --ignored --nocapture
//! ```

use std::sync::mpsc::channel;
use std::sync::{Mutex, MutexGuard};

/// The server keeps one session per task, and refuses a request for a session
/// that is already in flight. Both tests here use the commit session, so they
/// take turns rather than run in parallel.
static SERVER: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    SERVER.lock().unwrap_or_else(|err| err.into_inner())
}

use lg::state::GenMsg;

#[test]
#[ignore = "needs a model server on the configured endpoint"]
fn a_real_request_comes_back_with_prefill_and_decode_throughput() {
    let _turn = take_turn();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        lg::llm::stream_commit_message(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n"
                .to_string(),
            tx,
        );
    });

    let mut stats = None;
    let mut error = None;
    for msg in rx {
        match msg {
            GenMsg::Done {
                stats: reported, ..
            } => stats = Some(reported),
            GenMsg::Error(message) => error = Some(message),
            _ => {}
        }
    }

    assert_eq!(error, None, "the server refused the request");
    let stats = stats.expect("the stream reported no stats at all");
    println!("{stats:#?}");
    assert!(
        stats.prefill_tps > 0.0,
        "no prefill rate: {stats:?} \u{2014} the server sent no usage chunk"
    );
    assert!(stats.decode_tps > 0.0, "no decode rate: {stats:?}");
    assert!(stats.completion_tokens > 0, "no token counts: {stats:?}");
    assert!(
        stats.served_model.is_some(),
        "the server did not say which model answered: {stats:?}"
    );
}

/// The prefill cache is what makes a large, stable prompt prefix affordable to
/// send on every commit. Two requests for the same task share that prefix, so
/// the second one should not be read from scratch.
#[test]
#[ignore = "needs a model server on the configured endpoint"]
fn a_second_request_for_the_same_task_reuses_its_prefill() {
    let _turn = take_turn();
    let diff = format!(
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n{}",
        "+new line\n".repeat(200)
    );
    let stats = |diff: String| {
        let (tx, rx) = channel();
        std::thread::spawn(move || lg::llm::stream_commit_message(diff, tx));
        rx.into_iter().find_map(|msg| match msg {
            GenMsg::Done { stats, .. } => Some(stats),
            _ => None,
        })
    };

    let first = stats(diff.clone()).expect("first request reported stats");
    let second = stats(diff).expect("second request reported stats");

    println!("first: {first:#?}\nsecond: {second:#?}");
    // Asserted on the second request alone rather than as a rise over the
    // first: the server's cache outlives the test, so a run that follows
    // another starts warm and there is no rise to see.
    assert!(
        second.cached_tokens > 0,
        "the shared prompt prefix was read from scratch: {second:?}"
    );
    assert!(
        second.cached_tokens * 2 > second.prompt_tokens,
        "most of a repeated prompt should come from the cache: {second:?}"
    );
}
