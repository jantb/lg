//! Answering through the `claude` CLI instead of a local chat endpoint.
//!
//! Each request is one `claude -p` run with every tool, MCP server and settings
//! file turned off and lg's own system prompt in place of Claude Code's: what
//! comes back is text, exactly as the local model would have written it. The
//! prompt goes in on stdin, because a branch-wide review is larger than an
//! argument list is allowed to be.

use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::state::GenMsg;

use super::stats::{GenStats, Tracked};
use super::stream::{ChatMessage, ChatTask, UNREACHABLE};

/// What lg says to Claude when a task brings no system prompt of its own.
const DEFAULT_SYSTEM_PROMPT: &str = "You are the text-generation backend of lg, a terminal git \
client. Answer with exactly what the request asks for, in the format it asks for. No preamble, \
no closing remarks, no questions back.";

/// The models offered for the Claude provider. The CLI resolves each alias to
/// the newest model of that family.
pub const CLAUDE_MODEL_CHOICES: &[&str] = &["sonnet", "opus", "haiku"];

/// Run one request through `claude -p` and report it on `tx` the way
/// [`super::stream::stream_messages`] reports a local one: thinking and output
/// chunks as they arrive, then exactly one [`GenMsg::Done`] or
/// [`GenMsg::Error`].
pub fn stream_claude(
    messages: Vec<ChatMessage>,
    task: ChatTask,
    finalizer: impl Fn(&str) -> String,
    tx: Sender<GenMsg>,
) {
    let (system, prompt) = split_messages(messages);
    let mut tracked = Tracked::new(system.len() + prompt.len());
    // A file rather than a pipe: nothing reads stderr until the run is over,
    // and a pipe that fills meanwhile would stall claude mid-answer.
    let mut stderr = tempfile::tempfile().ok();
    let mut child = match spawn(&system, task, stderr.as_ref()) {
        Ok(child) => child,
        Err(e) => {
            let _ = tx.send(GenMsg::Error(format!("claude {UNREACHABLE}: {e}")));
            return;
        }
    };
    // Written from its own thread: claude starts streaming before it has read
    // all of a long prompt, and a full stdout pipe would otherwise stall both
    // ends.
    let writer = child.stdin.take().map(|mut stdin| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        })
    });
    let Some(stdout) = child.stdout.take() else {
        let _ = tx.send(GenMsg::Error("claude: no output pipe".to_string()));
        stop(&mut child);
        return;
    };
    let end = consume_claude_stream(
        BufReader::new(stdout).lines(),
        &finalizer,
        &tx,
        &mut tracked,
    );
    match end {
        StreamEnd::Answered => {
            let _ = child.wait();
        }
        StreamEnd::Abandoned => stop(&mut child),
        StreamEnd::Closed => {
            let status = child.wait().ok();
            report_early_exit(&tx, status, stderr.as_mut());
        }
    }
    if let Some(writer) = writer {
        let _ = writer.join();
    }
}

/// How a run's stream came to an end.
#[derive(Debug, PartialEq, Eq)]
enum StreamEnd {
    /// The `result` event came, and the receiver has its ending.
    Answered,
    /// The receiver went away; the run is no longer wanted.
    Abandoned,
    /// Stdout closed without a `result`: the receiver is still owed an ending.
    Closed,
}

/// The process failed before it could say why on stdout, so stderr is the only
/// account of it.
fn report_early_exit(
    tx: &Sender<GenMsg>,
    status: Option<std::process::ExitStatus>,
    stderr: Option<&mut std::fs::File>,
) {
    let mut detail = String::new();
    if let Some(file) = stderr
        && file.rewind().is_ok()
    {
        let _ = file.take(4096).read_to_string(&mut detail);
    }
    let detail = detail.trim();
    let message = match (status, detail.is_empty()) {
        (_, false) => format!("claude: {}", last_lines(detail, 3)),
        (Some(status), true) => format!("claude exited without an answer ({status})"),
        (None, true) => "claude exited without an answer".to_string(),
    };
    let _ = tx.send(GenMsg::Error(message));
}

fn spawn(system: &str, task: ChatTask, stderr: Option<&std::fs::File>) -> std::io::Result<Child> {
    let agent = crate::agents::claude_profile();
    let program = crate::agents::resolve(&agent.executable).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} was not found on PATH", agent.executable),
        )
    })?;
    let mut command = Command::new(program);
    for marker in crate::session::nested_claude_markers() {
        command.env_remove(marker);
    }
    command.args(claude_args(
        system,
        task,
        &super::provider::configured_claude_model(),
    ));
    // Nowhere in particular: no tool can read the checkout, and a directory
    // without a CLAUDE.md keeps someone else's instructions out of the answer.
    command
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(match stderr.map(std::fs::File::try_clone) {
            Some(Ok(file)) => Stdio::from(file),
            _ => Stdio::null(),
        });
    command.spawn()
}

/// The arguments for one tool-free, settings-free, unrecorded print run.
fn claude_args(system: &str, task: ChatTask, model: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--system-prompt",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    args.push(system.to_string());
    if !model.is_empty() {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    // A task that wants an answer rather than deliberation gets as little
    // reasoning as the CLI allows; the rest keep its default.
    if !task.thinking {
        args.extend(["--effort".to_string(), "low".to_string()]);
    }
    args
}

/// The tools a fix run may use: enough to read the code and change it, and
/// nothing that runs a command.
const FIX_TOOLS: &str = "Read,Edit,Write,Grep,Glob";

/// Have Claude Code apply one change in `cwd` and wait for it, returning the
/// one line it says about what it did.
///
/// The run edits files and does nothing else: no shell, no MCP servers, no
/// settings files with hooks of their own, and edits accepted without asking
/// because nobody is at its terminal to ask. Blocks, so it belongs on a thread.
pub fn run_claude_fix(cwd: &std::path::Path, prompt: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    let agent = crate::agents::claude_profile();
    let program = crate::agents::resolve(&agent.executable)
        .with_context(|| format!("{} was not found on PATH", agent.executable))?;
    let mut command = Command::new(program);
    for marker in crate::session::nested_claude_markers() {
        command.env_remove(marker);
    }
    command.args([
        "-p",
        "--output-format",
        "json",
        "--tools",
        FIX_TOOLS,
        "--allowedTools",
        FIX_TOOLS,
        "--permission-mode",
        "acceptEdits",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
    ]);
    let model = super::provider::configured_claude_model();
    if !model.is_empty() {
        command.args(["--model", &model]);
    }
    let mut child = command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start claude")?;
    if let Some(mut stdin) = child.stdin.take() {
        let prompt = prompt.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
    }
    let out = child.wait_with_output().context("wait for claude")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let result: Option<serde_json::Value> = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok());
    match result {
        Some(result) if result.get("is_error").and_then(|e| e.as_bool()) != Some(true) => {
            Ok(result
                .get("result")
                .and_then(|r| r.as_str())
                .map(|r| last_lines(r.trim(), 1))
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "claude applied the change".to_string()))
        }
        Some(result) => anyhow::bail!(
            "claude: {}",
            result
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("the fix failed")
        ),
        None => anyhow::bail!(
            "claude exited without an answer: {}",
            last_lines(String::from_utf8_lossy(&out.stderr).trim(), 3)
        ),
    }
}

/// The system prompt, and everything else folded into one user turn.
///
/// `claude -p` takes a single prompt, so an earlier exchange travels as a
/// transcript ahead of the question it leads up to.
fn split_messages(messages: Vec<ChatMessage>) -> (String, String) {
    let mut system = Vec::new();
    let mut turns = Vec::new();
    for message in messages {
        if message.role == "system" {
            system.push(message.content);
        } else {
            turns.push(message);
        }
    }
    let system = if system.is_empty() {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        system.join("\n\n")
    };
    let Some(last) = turns.pop() else {
        return (system, String::new());
    };
    if turns.is_empty() {
        return (system, last.content);
    }
    let mut prompt = String::from("The conversation so far:\n\n");
    for turn in turns {
        let speaker = if turn.role == "assistant" {
            "Assistant"
        } else {
            "User"
        };
        prompt.push_str(&format!("{speaker}: {}\n\n", turn.content.trim()));
    }
    prompt.push_str("Answer the user's next message:\n\n");
    prompt.push_str(&last.content);
    (system, prompt)
}

/// Read claude's stream-json events, sending chunks as they arrive and ending
/// with exactly one [`GenMsg::Done`] or [`GenMsg::Error`] once the `result`
/// event comes.
fn consume_claude_stream(
    lines: impl Iterator<Item = std::io::Result<String>>,
    finalizer: &dyn Fn(&str) -> String,
    tx: &Sender<GenMsg>,
    tracked: &mut Tracked,
) -> StreamEnd {
    let started = Instant::now();
    let mut text = String::new();
    let mut model = None;
    for line in lines {
        let Ok(line) = line else {
            return StreamEnd::Closed;
        };
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        match event.get("type").and_then(|kind| kind.as_str()) {
            Some("system") => {
                if let Some(name) = event.get("model").and_then(|m| m.as_str()) {
                    model = Some(name.to_string());
                }
            }
            Some("stream_event") => {
                let Some(delta) = event.pointer("/event/delta") else {
                    continue;
                };
                let (chunk, thinking) = match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => (delta.get("text"), false),
                    Some("thinking_delta") => (delta.get("thinking"), true),
                    _ => continue,
                };
                let Some(chunk) = chunk.and_then(|c| c.as_str()).filter(|c| !c.is_empty()) else {
                    continue;
                };
                tracked.note_token();
                let sent = if thinking {
                    tx.send(GenMsg::Thinking(chunk.to_string()))
                } else {
                    text.push_str(chunk);
                    tx.send(GenMsg::Output(chunk.to_string()))
                };
                if sent.is_err() {
                    return StreamEnd::Abandoned;
                }
            }
            Some("result") => {
                if event.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
                    let reason = event
                        .get("result")
                        .and_then(|r| r.as_str())
                        .filter(|r| !r.trim().is_empty())
                        .or_else(|| event.get("subtype").and_then(|s| s.as_str()))
                        .unwrap_or("request failed");
                    let _ = tx.send(GenMsg::Error(format!("claude: {}", reason.trim())));
                    return StreamEnd::Answered;
                }
                if let Some(result) = event.get("result").and_then(|r| r.as_str())
                    && !result.trim().is_empty()
                {
                    text = result.to_string();
                }
                let stats = result_stats(&event, model.take(), started);
                tracked.report(stats.clone());
                let _ = tx.send(GenMsg::Done {
                    text: finalizer(&text),
                    stats,
                });
                return StreamEnd::Answered;
            }
            _ => {}
        }
    }
    StreamEnd::Closed
}

/// What the `result` event says the run cost.
fn result_stats(event: &serde_json::Value, model: Option<String>, started: Instant) -> GenStats {
    let usage = |key: &str| {
        event
            .pointer(&format!("/usage/{key}"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let cached = usage("cache_read_input_tokens");
    let prompt_tokens = usage("input_tokens") + usage("cache_creation_input_tokens") + cached;
    let completion_tokens = usage("output_tokens");
    let millis = |key: &str| event.get(key).and_then(serde_json::Value::as_u64);
    let total_ms = millis("duration_ms").unwrap_or_else(|| started.elapsed().as_millis() as u64);
    let first_ms = millis("ttft_ms").unwrap_or(0).min(total_ms);
    let rate = |tokens: u64, ms: u64| {
        if ms == 0 {
            0.0
        } else {
            tokens as f64 * 1000.0 / ms as f64
        }
    };
    let served_model = event
        .get("modelUsage")
        .and_then(|usage| usage.as_object())
        .and_then(|usage| usage.keys().next().cloned())
        .or(model);
    GenStats {
        prompt_tokens,
        cached_tokens: cached,
        completion_tokens,
        prefill_tps: rate(prompt_tokens, first_ms),
        decode_tps: rate(completion_tokens, total_ms - first_ms),
        truncated: event.get("stop_reason").and_then(|r| r.as_str()) == Some("max_tokens"),
        served_model,
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" / ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn drive(lines: &[&str]) -> (StreamEnd, Vec<GenMsg>) {
        let (tx, rx) = channel();
        let ended = consume_claude_stream(
            lines.iter().map(|line| Ok(line.to_string())),
            &|raw: &str| raw.trim().to_string(),
            &tx,
            &mut Tracked::new(0),
        );
        drop(tx);
        (ended, rx.iter().collect())
    }

    #[test]
    fn text_deltas_stream_as_output_and_the_result_ends_the_answer() {
        let (ended, msgs) = drive(&[
            r#"{"type":"system","subtype":"init","model":"claude-sonnet-5-5"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"weighing"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"feat: "}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"add a thing"}}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"feat: add a thing\n","stop_reason":"end_turn","duration_ms":2000,"ttft_ms":500,"usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":30}}"#,
        ]);

        assert_eq!(ended, StreamEnd::Answered);
        assert!(matches!(&msgs[0], GenMsg::Thinking(s) if s == "weighing"));
        assert!(matches!(&msgs[1], GenMsg::Output(s) if s == "feat: "));
        let Some(GenMsg::Done { text, stats }) = msgs.last() else {
            panic!("the answer ends with Done: {msgs:?}");
        };
        assert_eq!(text, "feat: add a thing");
        assert_eq!(stats.prompt_tokens, 100);
        assert_eq!(stats.cached_tokens, 90);
        assert_eq!(stats.completion_tokens, 30);
        assert_eq!(stats.served_model.as_deref(), Some("claude-sonnet-5-5"));
        assert!(!stats.truncated);
    }

    #[test]
    fn a_failed_run_is_reported_as_an_error() {
        let (ended, msgs) = drive(&[
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Not logged in"}"#,
        ]);

        assert_eq!(ended, StreamEnd::Answered);
        assert!(matches!(&msgs[..], [GenMsg::Error(s)] if s.contains("Not logged in")));
    }

    #[test]
    fn a_stream_that_closes_without_a_result_is_left_to_the_caller() {
        let (ended, msgs) = drive(&[
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"half"}}}"#,
        ]);

        assert_eq!(ended, StreamEnd::Closed);
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m, GenMsg::Done { .. } | GenMsg::Error(_)))
        );
    }

    #[test]
    fn an_answer_stopped_at_the_output_limit_is_truncated() {
        let (_, msgs) = drive(&[
            r#"{"type":"result","is_error":false,"result":"half a","stop_reason":"max_tokens"}"#,
        ]);

        assert!(matches!(msgs.last(), Some(GenMsg::Done { stats, .. }) if stats.truncated));
    }

    #[test]
    fn earlier_turns_travel_ahead_of_the_question() {
        let (system, prompt) = split_messages(vec![
            ChatMessage {
                role: "system",
                content: "Review this.".into(),
            },
            ChatMessage {
                role: "user",
                content: "Is it safe?".into(),
            },
            ChatMessage {
                role: "assistant",
                content: "Mostly.".into(),
            },
            ChatMessage {
                role: "user",
                content: "What about the unwrap?".into(),
            },
        ]);

        assert_eq!(system, "Review this.");
        assert!(prompt.contains("Is it safe?") && prompt.contains("Mostly."));
        assert!(prompt.trim_end().ends_with("What about the unwrap?"));
    }

    #[test]
    fn a_single_prompt_goes_as_it_is_under_the_default_system_prompt() {
        let (system, prompt) = split_messages(vec![ChatMessage {
            role: "user",
            content: "Write a commit message.".into(),
        }]);

        assert_eq!(system, DEFAULT_SYSTEM_PROMPT);
        assert_eq!(prompt, "Write a commit message.");
    }

    #[test]
    fn a_run_has_no_tools_and_reads_no_settings() {
        let args = claude_args(
            "sys",
            ChatTask {
                session: "lg-test",
                num_predict: 0,
                thinking: false,
            },
            "opus",
        );
        let after = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .map(|i| args[i + 1].as_str())
        };

        assert_eq!(after("--tools"), Some(""));
        assert_eq!(after("--setting-sources"), Some(""));
        assert_eq!(after("--model"), Some("opus"));
        assert_eq!(after("--system-prompt"), Some("sys"));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
    }
}
