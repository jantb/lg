use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::config::{
    COMMIT_MSG_GEN_MAX_CHARS, COMMIT_MSG_SUBJECT_MAX_CHARS, COMMIT_PROMPT_PREFIX,
    LLAMA_SERVER_CHAT_ENDPOINT, LLM_MODEL, LLM_NUM_PREDICT, LLM_TEMPERATURE, LLM_TOP_P,
};
use crate::state::{GenMsg, ReviewChatMessage, ReviewStyleFinding, ReviewStyleSeverity};

const MAX_DIFF_EXCERPT_LINES: usize = 180;
const MAX_DIFF_EXCERPT_BYTES: usize = 16_000;
const MAX_SUMMARY_FILES: usize = 24;
const MAX_SIGNAL_LINES: usize = 48;
const REVIEW_ASSIST_MAX_CHARS: usize = 2_400;
const REVIEW_CHAT_MAX_CHARS: usize = 12_000;
const CONFIG_FILE_ENV: &str = "LG_CONFIG_FILE";
const CONFIG_MODEL_KEY: &str = "llm_model";
const CONFIG_PROVIDER_KEY: &str = "llm_provider";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    LlamaServer,
}

impl LlmProvider {
    pub const ALL: [Self; 1] = [Self::LlamaServer];

    pub fn label(self) -> &'static str {
        "llama-server"
    }

    fn config_value(self) -> &'static str {
        "llama-server"
    }

    fn from_config(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "llama" | "llama-server" | "llama_server" | "llamacpp" | "llama.cpp" => {
                Some(Self::LlamaServer)
            }
            _ => None,
        }
    }

    fn default_endpoint(self) -> &'static str {
        LLAMA_SERVER_CHAT_ENDPOINT
    }

    fn endpoint_env(self) -> Option<String> {
        std::env::var("LG_LLAMA_SERVER_CHAT_ENDPOINT")
            .or_else(|_| std::env::var("LG_LLAMA_SERVER_URL"))
            .ok()
            .map(|endpoint| endpoint.trim().to_string())
            .filter(|endpoint| !endpoint.is_empty())
            .map(|endpoint| normalize_llama_server_chat_endpoint(&endpoint))
    }
}

#[derive(Serialize)]
struct LlamaServerChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    stream: bool,
    temperature: f32,
    top_p: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i32>,
    chat_template_kwargs: ChatTemplateKwargs,
}

#[derive(Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
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
    num_predict: i32,
}

impl Default for Options {
    fn default() -> Self {
        let num_predict = std::env::var("LG_LLM_NUM_PREDICT")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(LLM_NUM_PREDICT);

        Self {
            temperature: LLM_TEMPERATURE,
            top_p: LLM_TOP_P,
            num_predict,
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
        .unwrap_or_else(|| LLM_MODEL.to_owned())
}

pub fn current_provider() -> LlmProvider {
    env_provider()
        .or_else(saved_provider)
        .unwrap_or(LlmProvider::LlamaServer)
}

pub fn current_endpoint() -> String {
    endpoint_for_provider(current_provider())
}

pub fn endpoint_for_provider(provider: LlmProvider) -> String {
    provider
        .endpoint_env()
        .unwrap_or_else(|| provider.default_endpoint().to_owned())
}

fn normalize_llama_server_chat_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.ends_with("/v1/chat/completions") {
        endpoint.to_string()
    } else if endpoint.ends_with("/v1") {
        format!("{endpoint}/chat/completions")
    } else {
        format!("{endpoint}/v1/chat/completions")
    }
}

pub fn env_model_active() -> bool {
    env_model().is_some()
}

pub fn env_provider_active() -> bool {
    env_provider().is_some()
}

pub fn config_file_display() -> String {
    config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "$HOME/.config/lg/config".to_string())
}

pub fn save_model(model: &str) -> Result<()> {
    save_llm_settings(model, current_provider())
}

pub fn save_llm_settings(model: &str, provider: LlmProvider) -> Result<()> {
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
    set_config_entry(&mut entries, CONFIG_PROVIDER_KEY, provider.config_value());
    fs::write(&path, render_config_entries(&entries))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn clear_saved_model() -> Result<()> {
    clear_saved_llm_settings()
}

pub fn clear_saved_llm_settings() -> Result<()> {
    let path = config_path()?;
    let mut entries = read_config_entries(&path);
    let before = entries.len();
    entries.retain(|(key, _)| key != CONFIG_MODEL_KEY && key != CONFIG_PROVIDER_KEY);
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
    std::env::var("LG_LLM_MODEL")
        .ok()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn env_provider() -> Option<LlmProvider> {
    std::env::var("LG_LLM_PROVIDER")
        .ok()
        .and_then(|provider| LlmProvider::from_config(&provider))
}

fn saved_model() -> Option<String> {
    let path = config_path().ok()?;
    read_config_entries(&path)
        .into_iter()
        .find_map(|(key, value)| (key == CONFIG_MODEL_KEY && !value.is_empty()).then_some(value))
}

fn saved_provider() -> Option<LlmProvider> {
    let path = config_path().ok()?;
    read_config_entries(&path)
        .into_iter()
        .find_map(|(key, value)| {
            (key == CONFIG_PROVIDER_KEY)
                .then(|| LlmProvider::from_config(&value))
                .flatten()
        })
}

fn config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(CONFIG_FILE_ENV)
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
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
        "{COMMIT_PROMPT_PREFIX}{}\n\nDiff excerpt:\n{}\n",
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

/// Stream tokens from the local llama-server OpenAI-compatible chat endpoint,
/// routing reasoning chunks (and any inline `<think>...</think>` content) to
/// [`GenMsg::Thinking`], and content chunks to [`GenMsg::Output`].
/// Ends with a [`GenMsg::Done`] or [`GenMsg::Error`].
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
            role: message.role.as_chat_role(),
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
    if std::env::var_os("LG_LLM_NUM_PREDICT").is_none() {
        opts.num_predict = 256;
    }
    opts
}

fn review_chat_options() -> Options {
    let mut opts = Options::default();
    if std::env::var_os("LG_LLM_NUM_PREDICT").is_none() {
        opts.num_predict = 768;
    }
    opts
}

fn review_style_flag_options() -> Options {
    let mut opts = Options::default();
    if std::env::var_os("LG_LLM_NUM_PREDICT").is_none() {
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
    let provider = current_provider();
    let endpoint = endpoint_for_provider(provider);
    let prompt_bytes = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();
    let num_predict = opts.num_predict;

    let body = match chat_request_body(&model, messages, opts) {
        Ok(body) => body,
        Err(e) => {
            let _ = tx.send(GenMsg::Error(format!("llm request body: {e}")));
            return;
        }
    };

    let mut trace = std::env::var_os("LG_LLM_TRACE")
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());

    if let Some(f) = trace.as_mut() {
        let _ = writeln!(
            f,
            "# START provider={} model={} endpoint={} num_predict={} prompt_bytes={} elapsed_ms=0",
            provider.label(),
            model,
            endpoint,
            num_predict,
            prompt_bytes,
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
                let _ = writeln!(f, "# ERROR {} request: {e}", provider.label());
            }
            let _ = tx.send(GenMsg::Error(format!("{} request: {e}", provider.label())));
            return;
        }
    };
    let resp = match resp.error_for_status() {
        Ok(r) => r,
        Err(e) => {
            if let Some(f) = trace.as_mut() {
                let _ = writeln!(f, "# ERROR {} status: {e}", provider.label());
            }
            let _ = tx.send(GenMsg::Error(format!("{} status: {e}", provider.label())));
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
        let Some(json_line) = stream_json_line(&line) else {
            let trimmed = line.trim();
            if trimmed == "data: [DONE]" {
                let (tb, ob) = parser.flush(&tx, &mut full_output).unwrap_or((0, 0));
                think_bytes += tb;
                out_bytes += ob;
                send_done(
                    &mut trace,
                    finalizer,
                    &full_output,
                    DoneStats {
                        reason: "sse_done",
                        eval_count: 0,
                        prompt_eval_count: 0,
                        total_ms: 0,
                        eval_ms: 0,
                        think_bytes,
                        out_bytes,
                    },
                    &tx,
                );
                return;
            }
            continue;
        };
        let v: serde_json::Value = match serde_json::from_str(json_line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(t) = stream_thinking_chunk(&v) {
            think_bytes += t.len();
            if tx.send(GenMsg::Thinking(t.to_owned())).is_err() {
                return;
            }
        }
        if let Some(c) = stream_output_chunk(&v) {
            let (tb, ob) = match parser.feed(c, &tx, &mut full_output) {
                Ok(counts) => counts,
                Err(()) => return,
            };
            think_bytes += tb;
            out_bytes += ob;
        }
    }
    let (tb, ob) = parser.flush(&tx, &mut full_output).unwrap_or((0, 0));
    think_bytes += tb;
    out_bytes += ob;
    send_done(
        &mut trace,
        finalizer,
        &full_output,
        DoneStats {
            reason: "loop_exhausted",
            eval_count: 0,
            prompt_eval_count: 0,
            total_ms: 0,
            eval_ms: 0,
            think_bytes,
            out_bytes,
        },
        &tx,
    );
}

fn chat_request_body(
    model: &str,
    messages: Vec<ChatMessage>,
    opts: Options,
) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(LlamaServerChatRequest {
        model,
        messages,
        stream: true,
        temperature: opts.temperature,
        top_p: opts.top_p,
        max_tokens: (opts.num_predict > 0).then_some(opts.num_predict),
        chat_template_kwargs: ChatTemplateKwargs {
            enable_thinking: false,
        },
    })?)
}

fn stream_json_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed
        .strip_prefix("data:")
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "[DONE]")
}

fn stream_thinking_chunk(v: &serde_json::Value) -> Option<&str> {
    v.pointer("/choices/0/delta/reasoning_content")
        .or_else(|| v.pointer("/choices/0/delta/thinking"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

fn stream_output_chunk(v: &serde_json::Value) -> Option<&str> {
    v.pointer("/choices/0/delta/content")
        .or_else(|| v.pointer("/choices/0/message/content"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

struct DoneStats<'a> {
    reason: &'a str,
    eval_count: u64,
    prompt_eval_count: u64,
    total_ms: u64,
    eval_ms: u64,
    think_bytes: usize,
    out_bytes: usize,
}

fn send_done(
    trace: &mut Option<std::fs::File>,
    finalizer: fn(&str) -> String,
    full_output: &str,
    stats: DoneStats<'_>,
    tx: &Sender<GenMsg>,
) {
    let final_output = finalizer(full_output);
    if let Some(f) = trace.as_mut() {
        let _ = writeln!(
            f,
            "# DONE done_reason={} eval_count={} prompt_eval_count={} total_duration_ms={} eval_duration_ms={} think_bytes={} out_bytes={} final_output={final_output:?}",
            stats.reason,
            stats.eval_count,
            stats.prompt_eval_count,
            stats.total_ms,
            stats.eval_ms,
            stats.think_bytes,
            stats.out_bytes,
        );
    }
    let _ = tx.send(GenMsg::Done(final_output));
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
        .map(clean_review_style_line)
        .filter(|line| !line.is_empty())
    {
        let lower = line.to_ascii_lowercase();
        if let Some((_, value)) = lower
            .starts_with("severity:")
            .then(|| line.split_once(':'))
            .flatten()
        {
            if let Some(parsed) = parse_review_style_severity(value) {
                severity = Some(parsed);
                reason = None;
            }
        } else if let Some((_, value)) = lower
            .starts_with("reason:")
            .then(|| line.split_once(':'))
            .flatten()
        {
            if severity.is_some() {
                reason = Some(value.trim().to_string());
            }
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

fn clean_review_style_line(line: &str) -> &str {
    line.trim()
        .trim_start_matches(|ch: char| {
            ch.is_ascii_whitespace()
                || ch == '-'
                || ch == '*'
                || ch == '>'
                || ch == '`'
                || ch == '"'
                || ch == '\''
        })
        .trim_end_matches(['`', '"', '\''])
        .trim()
}

fn parse_review_style_severity(s: &str) -> Option<ReviewStyleSeverity> {
    let upper = s
        .trim()
        .trim_matches(|ch: char| {
            ch.is_ascii_whitespace()
                || ch == '`'
                || ch == '"'
                || ch == '\''
                || ch == '.'
                || ch == ':'
        })
        .to_ascii_uppercase();
    match upper.as_str() {
        "FAIL" | "RED" => Some(ReviewStyleSeverity::Fail),
        "WARN" | "WARNING" | "FLAG" => Some(ReviewStyleSeverity::Warn),
        "OK" | "GREEN" => Some(ReviewStyleSeverity::Ok),
        _ => None,
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
    fn llm_provider_parses_llama_server_aliases() {
        assert_eq!(
            LlmProvider::from_config("llama-server"),
            Some(LlmProvider::LlamaServer)
        );
        assert_eq!(
            LlmProvider::from_config("llama.cpp"),
            Some(LlmProvider::LlamaServer)
        );
        assert_eq!(LlmProvider::from_config("unsupported"), None);
    }

    #[test]
    fn llama_server_url_env_accepts_base_url() {
        assert_eq!(
            normalize_llama_server_chat_endpoint("http://localhost:3636"),
            "http://localhost:3636/v1/chat/completions"
        );
        assert_eq!(
            normalize_llama_server_chat_endpoint("http://localhost:3636/v1/chat/completions"),
            "http://localhost:3636/v1/chat/completions"
        );
        assert_eq!(
            normalize_llama_server_chat_endpoint("http://localhost:3636/v1"),
            "http://localhost:3636/v1/chat/completions"
        );
    }

    #[test]
    fn llama_server_request_uses_openai_chat_shape() {
        let opts = Options {
            num_predict: 42,
            ..Default::default()
        };
        let body = chat_request_body(
            "qwen-local",
            vec![ChatMessage {
                role: "user",
                content: "hi".into(),
            }],
            opts,
        )
        .unwrap();

        assert_eq!(body["model"], "qwen-local");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 42);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert!(body.get("keep_alive").is_none());
        assert!(body.get("options").is_none());
    }

    #[test]
    fn llama_server_stream_reads_sse_deltas() {
        let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let json = stream_json_line(line).unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();

        assert_eq!(stream_output_chunk(&value), Some("hello"));
        assert_eq!(stream_json_line("data: [DONE]"), None);
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
    fn review_style_parser_ignores_schema_when_reading_reasoning_fallback() {
        let finding = parse_review_style_finding(
            "Return exactly two lines:\n\
             `severity: OK|WARN|FAIL`\n\
             `reason: <one concise reason>`\n\n\
             Draft output:\n\
             severity: FAIL\n\
             reason: Controller contains business logic instead of delegating to a service.",
        );

        assert_eq!(finding.severity, ReviewStyleSeverity::Fail);
        assert_eq!(
            finding.reason,
            "Controller contains business logic instead of delegating to a service."
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
