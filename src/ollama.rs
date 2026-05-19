use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::config::{
    COMMIT_MSG_GEN_MAX_CHARS, COMMIT_MSG_SUBJECT_MAX_CHARS, OLLAMA_CHAT_ENDPOINT,
    OLLAMA_KEEP_ALIVE, OLLAMA_MIN_P, OLLAMA_MODEL, OLLAMA_NUM_CTX, OLLAMA_NUM_PREDICT,
    OLLAMA_PRESENCE_PENALTY, OLLAMA_PROMPT_PREFIX, OLLAMA_REPEAT_PENALTY, OLLAMA_TEMPERATURE,
    OLLAMA_TOP_K, OLLAMA_TOP_P,
};
use crate::state::{GenMsg, ReviewChatMessage, ReviewStyleFinding, ReviewStyleSeverity};

const MAX_DIFF_EXCERPT_LINES: usize = 180;
const MAX_DIFF_EXCERPT_BYTES: usize = 16_000;
const MAX_SUMMARY_FILES: usize = 24;
const MAX_SIGNAL_LINES: usize = 48;
const REVIEW_ASSIST_MAX_CHARS: usize = 2_400;
const REVIEW_CHAT_MAX_CHARS: usize = 12_000;
const CONFIG_FILE_ENV: &str = "LG_CONFIG_FILE";
const CONFIG_MODEL_KEY: &str = "ollama_model";
const REVIEW_REPO_STYLE_GUIDE: &str = "\
Established repo style:
- Kotlin/Spring, but immutable code by default: prefer val, immutable collections, data-class .copy(), focused functions, and pure helper functions.
- Constructor injection only. Inject narrow interfaces/services, not broad infrastructure.
- Controllers stay thin: auth, validation, DTO assembly, ResponseEntity. Business decisions go in service-layer files/classes whose path or name contains Service, or in explicit hub flow code.
- Treat business rules in controllers, adapters, Kafka consumers/listeners, repositories, DTOs, configuration, or other non-Service/non-flow files as a style issue unless the shown code only delegates or translates data.
- Domain IDs use inline value classes like UserId, MembershipId; wrap raw primitives at repository boundaries.
- Use sealed interfaces/classes for variants with different data; enums only for simple tags.
- JSON uses the shared configuredJson; avoid Jackson in app code except generated/Spring/Avro internals.
- Time uses kotlinx.datetime; java.time only at interop edges.
- Logging uses private val log by Logger(), not direct LoggerFactory.
- Outbound HTTP uses Ktor CIO adapters. Each external system gets one adapter.
- Persistence is PostgreSQL via Exposed + Flyway.
- Kafka/outbound side effects from flows go through the outbox, not direct Kafka publishing.
- Tests prefer real small fakes over mocks. Use Mockk only when a fake is impractical; never Mockito.
- Integration tests use @SpringBootTest + TestConfiguration + Testcontainers.
- Do not edit generated code under target/generated-sources.
- Run the repo formatter/lint before declaring work done; linter wins on formatting.";

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    stream: bool,
    think: bool,
    keep_alive: &'a str,
    options: Options,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct Options {
    temperature: f32,
    top_p: f32,
    top_k: u32,
    min_p: f32,
    num_ctx: u32,
    num_predict: i32,
    repeat_penalty: f32,
    presence_penalty: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<&'static str>,
}

impl Default for Options {
    fn default() -> Self {
        let num_predict = std::env::var("LG_OLLAMA_NUM_PREDICT")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(OLLAMA_NUM_PREDICT);

        Self {
            temperature: OLLAMA_TEMPERATURE,
            top_p: OLLAMA_TOP_P,
            top_k: OLLAMA_TOP_K,
            min_p: OLLAMA_MIN_P,
            num_ctx: OLLAMA_NUM_CTX,
            num_predict,
            repeat_penalty: OLLAMA_REPEAT_PENALTY,
            presence_penalty: OLLAMA_PRESENCE_PENALTY,
            stop: Vec::new(),
        }
    }
}

#[derive(Default)]
struct DiffFileSummary {
    path: String,
    added: usize,
    removed: usize,
    hunks: Vec<String>,
}

pub fn current_model() -> String {
    env_model()
        .or_else(saved_model)
        .unwrap_or_else(|| OLLAMA_MODEL.to_owned())
}

pub fn env_model_active() -> bool {
    env_model().is_some()
}

pub fn save_model(model: &str) -> Result<()> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("model is empty");
    }
    if model.chars().any(|ch| ch == '\n' || ch == '\r') {
        anyhow::bail!("model must fit on one line");
    }
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut entries = read_config_entries(&path);
    set_config_entry(&mut entries, CONFIG_MODEL_KEY, model);
    fs::write(&path, render_config_entries(&entries))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn clear_saved_model() -> Result<()> {
    let path = config_path()?;
    let mut entries = read_config_entries(&path);
    let before = entries.len();
    entries.retain(|(key, _)| key != CONFIG_MODEL_KEY);
    if entries.len() == before && !path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, render_config_entries(&entries))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn env_model() -> Option<String> {
    std::env::var("LG_OLLAMA_MODEL")
        .ok()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn saved_model() -> Option<String> {
    let path = config_path().ok()?;
    read_config_entries(&path)
        .into_iter()
        .find_map(|(key, value)| (key == CONFIG_MODEL_KEY && !value.is_empty()).then_some(value))
}

fn config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(CONFIG_FILE_ENV)
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("lg/config"));
    }
    let Some(home) = std::env::var_os("HOME") else {
        anyhow::bail!("HOME is not set");
    };
    Ok(PathBuf::from(home).join(".config/lg/config"))
}

fn read_config_entries(path: &std::path::Path) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn set_config_entry(entries: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = entries.iter_mut().find(|(candidate, _)| candidate == key) {
        *existing = value.to_string();
    } else {
        entries.push((key.to_string(), value.to_string()));
    }
}

fn render_config_entries(entries: &[(String, String)]) -> String {
    let mut out = String::new();
    for (key, value) in entries {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

fn build_commit_prompt(diff: &str) -> String {
    format!(
        "{OLLAMA_PROMPT_PREFIX}{}\n\nDiff excerpt:\n{}\n",
        summarize_diff(diff),
        diff_excerpt(diff)
    )
}

fn build_review_assist_prompt(context: &str) -> String {
    format!(
        "Explain what this selected subtree from a full diff against main does.\n\
         Be concise and factual. Focus on behavior, call flow, tests, and review risks.\n\
         Review the change against the established repo style below and call out concrete violations.\n\
         Output 3-6 bullets. Do not invent files or behavior not shown. Do not use code fences.\n\n\
         {REVIEW_REPO_STYLE_GUIDE}\n\n\
         Selected review subtree:\n{context}"
    )
}

fn build_review_chat_system_prompt(context: &str) -> String {
    format!(
        "You are a senior code reviewer helping inspect a full branch review against main.\n\
         Use only the supplied review context and the conversation. Be concrete about weaknesses,\n\
         missed tests, risky flows, compatibility concerns, and follow-up checks. When useful,\n\
         cite file paths, function names, and line numbers from the context. If the context is\n\
         insufficient, say what is missing instead of guessing. Review answers against the\n\
         established repo style below and call out concrete violations.\n\n\
         {REVIEW_REPO_STYLE_GUIDE}\n\n\
         Review context:\n{context}"
    )
}

fn build_review_style_flag_prompt(path: &str, context: &str) -> String {
    format!(
        "Review this single changed Kotlin file for concrete violations of the established repo style.\n\
         Return exactly two lines:\n\
         severity: OK|WARN|FAIL\n\
         reason: <one concise reason, or \"No style issue found.\">\n\n\
         Use OK for files that look consistent or where there is insufficient evidence.\n\
         Use WARN for likely style issues that deserve manual attention.\n\
         Use FAIL for clear violations such as business logic in controllers or other non-Service/non-flow files,\n\
         direct Kafka side effects, Jackson app-code usage, Mockito, java.time away from interop edges,\n\
         or generated code edits.\n\n\
         {REVIEW_REPO_STYLE_GUIDE}\n\n\
         File: {path}\n\
         Review context:\n{context}"
    )
}

fn summarize_diff(diff: &str) -> String {
    let mut files: Vec<DiffFileSummary> = Vec::new();
    let mut current: Option<usize> = None;
    let mut signals: Vec<String> = Vec::new();

    for line in diff.lines() {
        if let Some(path) = parse_diff_path(line) {
            files.push(DiffFileSummary {
                path,
                ..Default::default()
            });
            current = Some(files.len() - 1);
            continue;
        }

        if let Some(i) = current {
            if line.starts_with("@@") {
                if files[i].hunks.len() < 3 {
                    files[i].hunks.push(truncate_line(line, 90));
                }
            } else if line.starts_with('+') && !line.starts_with("+++") {
                files[i].added += 1;
                push_signal(&mut signals, '+', line);
            } else if line.starts_with('-') && !line.starts_with("---") {
                files[i].removed += 1;
                push_signal(&mut signals, '-', line);
            }
        }
    }

    if files.is_empty() {
        return "No textual diff was found.".to_owned();
    }

    let mut out = String::new();
    out.push_str("Files changed:\n");
    for file in files.iter().take(MAX_SUMMARY_FILES) {
        out.push_str("- ");
        out.push_str(&file.path);
        out.push_str(&format!(" (+{} -{})", file.added, file.removed));
        if !file.hunks.is_empty() {
            out.push_str("; hunks: ");
            out.push_str(&file.hunks.join(" | "));
        }
        out.push('\n');
    }
    if files.len() > MAX_SUMMARY_FILES {
        out.push_str(&format!(
            "- ... {} more files\n",
            files.len() - MAX_SUMMARY_FILES
        ));
    }

    if !signals.is_empty() {
        out.push_str("\nNotable changed lines:\n");
        for line in signals {
            out.push_str("- ");
            out.push_str(&line);
            out.push('\n');
        }
    }

    out
}

fn diff_excerpt(diff: &str) -> String {
    let mut out = String::new();
    let mut bytes = 0usize;

    for (lines, line) in diff
        .lines()
        .filter(|line| is_excerpt_line(line))
        .enumerate()
    {
        let len = line.len() + 1;
        if lines >= MAX_DIFF_EXCERPT_LINES || bytes + len > MAX_DIFF_EXCERPT_BYTES {
            out.push_str("... diff excerpt truncated ...\n");
            break;
        }
        out.push_str(line);
        out.push('\n');
        bytes += len;
    }

    if out.trim().is_empty() {
        diff.lines()
            .take(40)
            .map(|line| truncate_line(line, 120))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        out
    }
}

fn parse_diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_, b_path) = rest.split_once(" b/")?;
    Some(b_path.to_owned())
}

fn push_signal(signals: &mut Vec<String>, prefix: char, line: &str) {
    if signals.len() >= MAX_SIGNAL_LINES {
        return;
    }
    let body = line[1..].trim();
    if body.is_empty() || matches!(body, "{" | "}" | ");" | "," | ")" | "]" | "};") {
        return;
    }
    signals.push(format!("{prefix} {}", truncate_line(body, 110)));
}

fn is_excerpt_line(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("@@")
        || (line.starts_with('+') && !line.starts_with("+++"))
        || (line.starts_with('-') && !line.starts_with("---"))
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

/// Stream tokens from Ollama's `/api/chat` endpoint, routing `message.thinking`
/// (and any inline `<think>…</think>` content in `message.content`) to
/// [`GenMsg::Thinking`], and the rest of `message.content` to
/// [`GenMsg::Output`]. Ends with a [`GenMsg::Done`] or [`GenMsg::Error`].
pub fn stream_commit_message(diff: String, tx: Sender<GenMsg>) {
    stream_prompt(build_commit_prompt(&diff), Options::default(), finalize, tx);
}

pub fn stream_review_assist(context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_review_assist_prompt(&context),
        review_assist_options(),
        finalize_review_assist,
        tx,
    );
}

pub fn stream_review_style_flag(path: String, context: String, tx: Sender<GenMsg>) {
    stream_prompt(
        build_review_style_flag_prompt(&path, &context),
        review_style_flag_options(),
        finalize_review_style_flag,
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
        content: build_review_chat_system_prompt(&context),
    }];
    for message in history
        .into_iter()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        messages.push(ChatMessage {
            role: message.role.as_ollama_role(),
            content: message.content,
        });
    }
    messages.push(ChatMessage {
        role: "user",
        content: prompt,
    });
    stream_messages(messages, review_chat_options(), finalize_review_chat, tx);
}

fn review_assist_options() -> Options {
    let mut opts = Options::default();
    if std::env::var_os("LG_OLLAMA_NUM_PREDICT").is_none() {
        opts.num_predict = 256;
    }
    opts
}

fn review_chat_options() -> Options {
    let mut opts = Options::default();
    if std::env::var_os("LG_OLLAMA_NUM_PREDICT").is_none() {
        opts.num_predict = 768;
    }
    opts
}

fn review_style_flag_options() -> Options {
    let mut opts = Options::default();
    if std::env::var_os("LG_OLLAMA_NUM_PREDICT").is_none() {
        opts.num_predict = 96;
    }
    opts
}

fn stream_prompt(prompt: String, opts: Options, finalizer: fn(&str) -> String, tx: Sender<GenMsg>) {
    stream_messages(
        vec![ChatMessage {
            role: "user",
            content: prompt,
        }],
        opts,
        finalizer,
        tx,
    );
}

fn stream_messages(
    messages: Vec<ChatMessage>,
    opts: Options,
    finalizer: fn(&str) -> String,
    tx: Sender<GenMsg>,
) {
    let start = Instant::now();
    let model = current_model();
    let endpoint = std::env::var("LG_OLLAMA_CHAT_ENDPOINT")
        .unwrap_or_else(|_| OLLAMA_CHAT_ENDPOINT.to_owned());
    let keep_alive =
        std::env::var("LG_OLLAMA_KEEP_ALIVE").unwrap_or_else(|_| OLLAMA_KEEP_ALIVE.to_owned());
    let prompt_bytes = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();

    let body = ChatRequest {
        model: &model,
        messages,
        stream: true,
        think: false,
        keep_alive: &keep_alive,
        options: opts,
    };

    let mut trace = std::env::var_os("LG_OLLAMA_TRACE")
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());

    if let Some(f) = trace.as_mut() {
        let _ = writeln!(
            f,
            "# START model={} think=false num_ctx={} num_predict={} prompt_bytes={} elapsed_ms=0",
            model, OLLAMA_NUM_CTX, body.options.num_predict, prompt_bytes,
        );
    }

    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            if let Some(f) = trace.as_mut() {
                let _ = writeln!(f, "# ERROR http client: {e}");
            }
            let _ = tx.send(GenMsg::Error(format!("http client: {e}")));
            return;
        }
    };

    let resp = match client.post(&endpoint).json(&body).send() {
        Ok(r) => r,
        Err(e) => {
            if let Some(f) = trace.as_mut() {
                let _ = writeln!(f, "# ERROR ollama request: {e}");
            }
            let _ = tx.send(GenMsg::Error(format!("ollama request: {e}")));
            return;
        }
    };
    let resp = match resp.error_for_status() {
        Ok(r) => r,
        Err(e) => {
            if let Some(f) = trace.as_mut() {
                let _ = writeln!(f, "# ERROR ollama status: {e}");
            }
            let _ = tx.send(GenMsg::Error(format!("ollama status: {e}")));
            return;
        }
    };

    let reader = BufReader::new(resp);
    let mut parser = ThinkSplit::default();
    let mut full_output = String::new();
    let mut think_bytes: usize = 0;
    let mut out_bytes: usize = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if let Some(f) = trace.as_mut() {
                    let _ = writeln!(
                        f,
                        "+T{} think_bytes={} out_bytes={} | # ERROR stream read: {e}",
                        start.elapsed().as_millis(),
                        think_bytes,
                        out_bytes,
                    );
                    let _ = writeln!(f, "# ERROR stream read: {e}");
                }
                let _ = tx.send(GenMsg::Error(format!("stream read: {e}")));
                return;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(f) = trace.as_mut() {
            let _ = writeln!(
                f,
                "+T{} think_bytes={} out_bytes={} | {}",
                start.elapsed().as_millis(),
                think_bytes,
                out_bytes,
                line,
            );
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg = v.get("message");
        if let Some(t) = msg
            .and_then(|m| m.get("thinking"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            think_bytes += t.len();
            if tx.send(GenMsg::Thinking(t.to_owned())).is_err() {
                return;
            }
        }
        if let Some(c) = msg
            .and_then(|m| m.get("content"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            let (tb, ob) = match parser.feed(c, &tx, &mut full_output) {
                Ok(counts) => counts,
                Err(()) => return,
            };
            think_bytes += tb;
            out_bytes += ob;
        }
        if v.get("done").and_then(|x| x.as_bool()).unwrap_or(false) {
            let (tb, ob) = parser.flush(&tx, &mut full_output).unwrap_or((0, 0));
            think_bytes += tb;
            out_bytes += ob;
            if let Some(f) = trace.as_mut() {
                let done_reason = v
                    .get("done_reason")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown");
                let eval_count = v.get("eval_count").and_then(|x| x.as_u64()).unwrap_or(0);
                let prompt_eval_count = v
                    .get("prompt_eval_count")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let total_ms = v
                    .get("total_duration")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0)
                    / 1_000_000;
                let eval_ms =
                    v.get("eval_duration").and_then(|x| x.as_u64()).unwrap_or(0) / 1_000_000;
                let _ = writeln!(
                    f,
                    "# DONE done_reason={done_reason} eval_count={eval_count} prompt_eval_count={prompt_eval_count} total_duration_ms={total_ms} eval_duration_ms={eval_ms} think_bytes={think_bytes} out_bytes={out_bytes} final_output={:?}",
                    finalizer(&full_output),
                );
            }
            let _ = tx.send(GenMsg::Done(finalizer(&full_output)));
            return;
        }
    }
    let (tb, ob) = parser.flush(&tx, &mut full_output).unwrap_or((0, 0));
    think_bytes += tb;
    out_bytes += ob;
    if let Some(f) = trace.as_mut() {
        let _ = writeln!(
            f,
            "# DONE done_reason=loop_exhausted eval_count=0 prompt_eval_count=0 total_duration_ms=0 eval_duration_ms=0 think_bytes={think_bytes} out_bytes={out_bytes} final_output={:?}",
            finalizer(&full_output),
        );
    }
    let _ = tx.send(GenMsg::Done(finalizer(&full_output)));
}

fn finalize(raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let mut lines: Vec<String> = cleaned
        .trim()
        .trim_matches('"')
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("```"))
        .map(trim_outer_quotes)
        .map(str::to_string)
        .collect();

    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    let Some(subject) = lines.first_mut() else {
        return String::new();
    };

    let (subject, _) = split_subject(subject);
    *lines.first_mut().expect("checked above") = subject;

    lines
        .join("\n")
        .chars()
        .take(COMMIT_MSG_GEN_MAX_CHARS)
        .collect()
}

fn finalize_review_assist(raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let mut lines = Vec::new();
    for line in cleaned
        .trim()
        .trim_matches('"')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("```"))
    {
        lines.push(trim_outer_quotes(line).to_string());
        if lines.len() >= 8 {
            break;
        }
    }
    lines
        .join("\n")
        .chars()
        .take(REVIEW_ASSIST_MAX_CHARS)
        .collect()
}

fn finalize_review_chat(raw: &str) -> String {
    strip_think_tags(raw)
        .trim()
        .trim_matches('"')
        .lines()
        .map(trim_outer_quotes_without_backticks)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(REVIEW_CHAT_MAX_CHARS)
        .collect()
}

fn finalize_review_style_flag(raw: &str) -> String {
    let cleaned = strip_think_tags(raw);
    let finding = parse_review_style_finding(&cleaned);
    format!(
        "severity: {}\nreason: {}",
        finding.severity.label(),
        finding.reason
    )
}

pub fn parse_review_style_finding(raw: &str) -> ReviewStyleFinding {
    let mut severity = None;
    let mut reason = None;
    for line in raw
        .trim()
        .trim_matches('"')
        .lines()
        .map(trim_outer_quotes_without_backticks)
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let lower = line.to_ascii_lowercase();
        if let Some((_, value)) = lower
            .starts_with("severity:")
            .then(|| line.split_once(':'))
            .flatten()
        {
            severity = parse_review_style_severity(value);
        } else if let Some((_, value)) = lower
            .starts_with("reason:")
            .then(|| line.split_once(':'))
            .flatten()
        {
            reason = Some(value.trim().to_string());
        } else if severity.is_none() {
            severity = parse_review_style_severity(line);
        } else if reason.is_none() {
            reason = Some(line.to_string());
        }
    }
    let severity = severity.unwrap_or(ReviewStyleSeverity::Ok);
    let reason = reason
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or_else(|| "No style issue found.".to_string());
    ReviewStyleFinding { severity, reason }
}

fn parse_review_style_severity(s: &str) -> Option<ReviewStyleSeverity> {
    let upper = s.trim().to_ascii_uppercase();
    if upper.contains("FAIL") || upper.contains("RED") {
        Some(ReviewStyleSeverity::Fail)
    } else if upper.contains("WARN") || upper.contains("WARNING") || upper.contains("FLAG") {
        Some(ReviewStyleSeverity::Warn)
    } else if upper.contains("OK") || upper.contains("GREEN") {
        Some(ReviewStyleSeverity::Ok)
    } else {
        None
    }
}

fn split_subject(s: &str) -> (String, String) {
    if s.chars().count() <= COMMIT_MSG_SUBJECT_MAX_CHARS {
        return (s.to_string(), String::new());
    }

    let split_at = s
        .char_indices()
        .take_while(|(i, _)| s[..*i].chars().count() <= COMMIT_MSG_SUBJECT_MAX_CHARS)
        .filter_map(|(i, c)| c.is_whitespace().then_some(i))
        .last()
        .unwrap_or_else(|| {
            s.char_indices()
                .nth(COMMIT_MSG_SUBJECT_MAX_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(s.len())
        });

    let subject = s[..split_at].trim().to_string();
    let overflow = s[split_at..].trim().to_string();
    (subject, overflow)
}

fn trim_outer_quotes(s: &str) -> &str {
    s.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
}

fn trim_outer_quotes_without_backticks(s: &str) -> &str {
    s.trim().trim_matches('"').trim_matches('\'')
}

fn strip_think_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("<think>") {
        out.push_str(&rest[..i]);
        rest = &rest[i + "<think>".len()..];
        if let Some(j) = rest.find("</think>") {
            rest = &rest[j + "</think>".len()..];
        } else {
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Default)]
struct ThinkSplit {
    in_think: bool,
    hold: String,
}

impl ThinkSplit {
    /// Feed a chunk; returns `Ok((think_bytes_added, out_bytes_added))`.
    fn feed(
        &mut self,
        chunk: &str,
        tx: &Sender<GenMsg>,
        full_output: &mut String,
    ) -> std::result::Result<(usize, usize), ()> {
        self.hold.push_str(chunk);
        let mut think_added = 0usize;
        let mut out_added = 0usize;
        loop {
            if self.in_think {
                if let Some(pos) = self.hold.find(CLOSE) {
                    let part: String = self.hold.drain(..pos).collect();
                    self.hold.drain(..CLOSE.len());
                    if !part.is_empty() {
                        think_added += part.len();
                        if tx.send(GenMsg::Thinking(part)).is_err() {
                            return Err(());
                        }
                    }
                    self.in_think = false;
                } else {
                    let keep = partial_tail_len(&self.hold, CLOSE);
                    let flush_len = self.hold.len() - keep;
                    if flush_len > 0 {
                        let flush: String = self.hold.drain(..flush_len).collect();
                        think_added += flush.len();
                        if tx.send(GenMsg::Thinking(flush)).is_err() {
                            return Err(());
                        }
                    }
                    return Ok((think_added, out_added));
                }
            } else if let Some(pos) = self.hold.find(OPEN) {
                let part: String = self.hold.drain(..pos).collect();
                self.hold.drain(..OPEN.len());
                if !part.is_empty() {
                    out_added += part.len();
                    full_output.push_str(&part);
                    if tx.send(GenMsg::Output(part)).is_err() {
                        return Err(());
                    }
                }
                self.in_think = true;
            } else {
                let keep = partial_tail_len(&self.hold, OPEN);
                let flush_len = self.hold.len() - keep;
                if flush_len > 0 {
                    let flush: String = self.hold.drain(..flush_len).collect();
                    out_added += flush.len();
                    full_output.push_str(&flush);
                    if tx.send(GenMsg::Output(flush)).is_err() {
                        return Err(());
                    }
                }
                return Ok((think_added, out_added));
            }
        }
    }

    /// Flush remaining held bytes; returns `Ok((think_bytes_added, out_bytes_added))`.
    fn flush(
        &mut self,
        tx: &Sender<GenMsg>,
        full_output: &mut String,
    ) -> std::result::Result<(usize, usize), ()> {
        if self.hold.is_empty() {
            return Ok((0, 0));
        }
        let tail: String = self.hold.drain(..).collect();
        let len = tail.len();
        if self.in_think {
            if tx.send(GenMsg::Thinking(tail)).is_err() {
                return Err(());
            }
            Ok((len, 0))
        } else {
            full_output.push_str(&tail);
            if tx.send(GenMsg::Output(tail)).is_err() {
                return Err(());
            }
            Ok((0, len))
        }
    }
}

fn partial_tail_len(s: &str, tag: &str) -> usize {
    let max = tag.len().saturating_sub(1).min(s.len());
    let sb = s.as_bytes();
    let tb = tag.as_bytes();
    for n in (1..=max).rev() {
        if sb.ends_with(&tb[..n]) {
            return n;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn finalize_strips_quotes_and_keeps_overflow() {
        assert_eq!(finalize("  \"feat: add\"  "), "feat: add");
        let long = "x".repeat(200);
        assert_eq!(finalize(&long), "x".repeat(COMMIT_MSG_SUBJECT_MAX_CHARS));
    }

    #[test]
    fn finalize_trims_long_subject_without_creating_body() {
        assert_eq!(
            finalize(
                "feat(tui): show a longer generated message that needs extra detail instead of being cut off"
            ),
            "feat(tui): show a longer generated message that needs extra detail"
        );
    }

    #[test]
    fn finalize_preserves_body_layout() {
        assert_eq!(
            finalize(
                "feat(tui): show active generation state\n\nAdds status counts.\nKeeps focused panels visible.\nKeeps the modal useful for longer messages.\nAvoids cutting off generated context.\nExtra line ignored."
            ),
            "feat(tui): show active generation state\n\nAdds status counts.\nKeeps focused panels visible.\nKeeps the modal useful for longer messages.\nAvoids cutting off generated context.\nExtra line ignored."
        );
    }

    #[test]
    fn strip_think_tags_removes_paired_blocks() {
        assert_eq!(
            strip_think_tags("before<think>planning</think>after"),
            "beforeafter"
        );
    }

    #[test]
    fn strip_think_tags_drops_unterminated_tail() {
        assert_eq!(strip_think_tags("keep<think>unterminated"), "keep");
    }

    #[test]
    fn review_assist_prompt_includes_repo_style() {
        let prompt = build_review_assist_prompt("src/main/kotlin/App.kt");

        assert!(prompt.contains("Constructor injection only"));
        assert!(prompt.contains("configuredJson"));
        assert!(prompt.contains("path or name contains Service"));
        assert!(prompt.contains("Selected review subtree:\nsrc/main/kotlin/App.kt"));
    }

    #[test]
    fn review_chat_system_prompt_includes_repo_style() {
        let prompt = build_review_chat_system_prompt("full review context");

        assert!(prompt.contains("Ktor CIO adapters"));
        assert!(prompt.contains("never Mockito"));
        assert!(prompt.contains("Review context:\nfull review context"));
    }

    #[test]
    fn review_style_flag_prompt_is_single_file() {
        let prompt =
            build_review_style_flag_prompt("src/main/kotlin/App.kt", "updates controller logic");

        assert!(prompt.contains("severity: OK|WARN|FAIL"));
        assert!(prompt.contains("non-Service/non-flow files"));
        assert!(prompt.contains("File: src/main/kotlin/App.kt"));
        assert!(prompt.contains("updates controller logic"));
    }

    #[test]
    fn finalize_review_style_flag_normalizes_output() {
        assert_eq!(
            finalize_review_style_flag("severity: WARN\nreason: controller does too much"),
            "severity: WARN\nreason: controller does too much"
        );
        assert_eq!(
            finalize_review_style_flag("FAIL\nDirect Kafka publish"),
            "severity: FAIL\nreason: Direct Kafka publish"
        );
        assert_eq!(
            finalize_review_style_flag("not enough evidence"),
            "severity: OK\nreason: No style issue found."
        );
    }

    #[test]
    fn partial_tail_detects_open_prefix() {
        assert_eq!(partial_tail_len("foo<thi", OPEN), 4);
        assert_eq!(partial_tail_len("done", OPEN), 0);
        assert_eq!(partial_tail_len("x<", OPEN), 1);
    }

    #[test]
    fn think_split_routes_inline_tags() {
        let (tx, rx) = channel::<GenMsg>();
        let mut p = ThinkSplit::default();
        let mut out = String::new();
        let (tb, ob) = p
            .feed("feat: foo<think>let me see</think> bar", &tx, &mut out)
            .unwrap();
        let (ftb, fob) = p.flush(&tx, &mut out).unwrap();
        drop(tx);
        let msgs: Vec<GenMsg> = rx.iter().collect();
        assert_eq!(out, "feat: foo bar");
        assert_eq!(tb + ftb, "let me see".len());
        assert_eq!(ob + fob, "feat: foo bar".len());
        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "feat: foo"));
        assert!(matches!(&msgs[1], GenMsg::Thinking(s) if s == "let me see"));
        assert!(matches!(&msgs[2], GenMsg::Output(s) if s == " bar"));
    }

    #[test]
    fn think_split_holds_partial_tag_across_chunks() {
        let (tx, rx) = channel::<GenMsg>();
        let mut p = ThinkSplit::default();
        let mut out = String::new();
        let (tb1, ob1) = p.feed("feat: foo<thi", &tx, &mut out).unwrap();
        let (tb2, ob2) = p.feed("nk>reason</think>ok", &tx, &mut out).unwrap();
        let (ftb, fob) = p.flush(&tx, &mut out).unwrap();
        drop(tx);
        let msgs: Vec<GenMsg> = rx.iter().collect();
        assert_eq!(out, "feat: foook");
        assert_eq!(tb1 + tb2 + ftb, "reason".len());
        assert_eq!(ob1 + ob2 + fob, "feat: foook".len());
        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "feat: foo"));
        assert!(matches!(&msgs[1], GenMsg::Thinking(s) if s == "reason"));
        assert!(matches!(&msgs[2], GenMsg::Output(s) if s == "ok"));
    }
}
