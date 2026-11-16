//! Running an external build command (make, just, docker) and streaming its
//! output to a log the caller can read back.
//!
//! Output goes to a log file first and is only summarized into the tool result,
//! so a command that outlives the MCP request — or floods stdout — is still
//! inspectable through `command_log_read`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};

use anyhow::anyhow;
use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::process::Command;

use crate::mcp::tools::inputs::require_tool_args;
use crate::mcp::tools::report::{
    MAX_OUTPUT_CHARS, format_output_block, new_output_log_path, output_dir, truncate_text,
};
use crate::mcp::tools::types::{ToolCallError, ToolCallResult, success_result};

pub(crate) const LONG_COMMAND_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60 * 60);
const MAX_COMMAND_TIMEOUT_SECS: u64 = 6 * 60 * 60;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandLogReadInput {
    /// Absolute log path returned by make_install, make_run, just_run, or
    /// docker_run, or a filename relative to this project's terrarium output
    /// directory.
    log_path: String,
    /// Number of lines to return from the end of the log. Default 200.
    #[schemars(range(min = 1))]
    tail_lines: Option<usize>,
    /// Optional Rust regex. When provided, only matching log lines are returned.
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    #[serde(default)]
    case_insensitive: Option<bool>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

pub(crate) fn command_log_read(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = require_tool_args::<CommandLogReadInput>(arguments)?;
    let log_path = resolve_command_log_path(project_root, &input.log_path)?;
    let regex = compile_output_regex(
        input.output_regex.as_deref(),
        input.case_insensitive.unwrap_or(false),
    )?;
    let text = std::fs::read_to_string(&log_path)
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to read command log: {err}")))?;
    let filtered = apply_output_regex(&text, regex.as_ref());
    let tailed = tail_lines(&filtered, input.tail_lines.unwrap_or(200));
    Ok(success_result(format!(
        "log: `{}`\n```text\n{}\n```",
        log_path.display(),
        truncate_text(&tailed, MAX_OUTPUT_CHARS)
    )))
}

pub(crate) async fn run_external_command(
    project_root: &Path,
    program: &str,
    args: &[String],
    output_regex: Option<&str>,
    case_insensitive: bool,
    timeout: std::time::Duration,
    background: bool,
) -> Result<ToolCallResult, ToolCallError> {
    let regex = compile_output_regex(output_regex, case_insensitive)?;
    let display_command = display_command(program, args);
    let log_path = command_output_path(project_root, program)?;
    prepare_command_log(&log_path, &display_command)?;
    if background {
        return spawn_background_command(project_root, program, args, &display_command, &log_path);
    }
    let stdout = open_command_log_stream(&log_path, "stdout")?;
    let stderr = stdout
        .try_clone()
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to open stderr log: {err}")))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(project_root)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let mut child = command
        .spawn()
        .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(result) => result.map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?,
        Err(_) => {
            let _ = child.start_kill();
            return Err(ToolCallError::Execution(anyhow!(
                "{display_command} timed out after {} seconds",
                timeout.as_secs()
            )));
        }
    };

    let log = std::fs::read_to_string(&log_path).map_err(|err| {
        ToolCallError::Execution(anyhow!(
            "failed to read command log after completion: {err}"
        ))
    })?;
    let command_output = output_from_log(&log);
    let combined = apply_output_regex(command_output, regex.as_ref());
    Ok(success_result(if status.success() {
        if combined.trim().is_empty() {
            format!("{display_command} succeeded\nlog: `{}`", log_path.display())
        } else {
            format!(
                "{display_command} succeeded\nlog: `{}`{}",
                log_path.display(),
                format_output_block(project_root, program, &combined)
            )
        }
    } else {
        format!(
            "{display_command} **failed**\nlog: `{}`{}",
            log_path.display(),
            format_output_block(project_root, program, &combined)
        )
    }))
}

fn spawn_background_command(
    project_root: &Path,
    program: &str,
    args: &[String],
    display_command: &str,
    log_path: &Path,
) -> Result<ToolCallResult, ToolCallError> {
    let stdout = open_command_log_stream(log_path, "background stdout")?;
    let stderr = stdout.try_clone().map_err(|err| {
        ToolCallError::Execution(anyhow!("failed to open background stderr log: {err}"))
    })?;

    let mut child = StdCommand::new(program)
        .args(args)
        .current_dir(project_root)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(success_result(format!(
        "{display_command} started in background\npid: {pid}\nlog: `{}`",
        log_path.display()
    )))
}

fn command_output_path(project_root: &Path, label: &str) -> Result<PathBuf, ToolCallError> {
    new_output_log_path(project_root, label).map_err(output_dir_error)
}

fn command_output_dir(project_root: &Path) -> Result<PathBuf, ToolCallError> {
    output_dir(project_root).map_err(output_dir_error)
}

fn output_dir_error(err: std::io::Error) -> ToolCallError {
    ToolCallError::Execution(anyhow!("failed to create command output directory: {err}"))
}

fn prepare_command_log(log_path: &Path, display_command: &str) -> Result<(), ToolCallError> {
    let mut header = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(log_path)
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to create command log: {err}")))?;
    writeln!(header, "$ {display_command}")
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to write command log: {err}")))?;
    writeln!(header)
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to write command log: {err}")))?;
    Ok(())
}

fn open_command_log_stream(log_path: &Path, stream: &str) -> Result<std::fs::File, ToolCallError> {
    std::fs::OpenOptions::new()
        .append(true)
        .open(log_path)
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to open {stream} log: {err}")))
}

/// Drops the `$ command` header [`prepare_command_log`] wrote, leaving what the
/// command itself produced.
fn output_from_log(log: &str) -> &str {
    log.split_once("\n\n")
        .map(|(_, output)| output)
        .unwrap_or(log)
}

fn resolve_command_log_path(project_root: &Path, log_path: &str) -> Result<PathBuf, ToolCallError> {
    let requested = Path::new(log_path);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        command_output_dir(project_root)?.join(requested)
    };
    let resolved = candidate.canonicalize().map_err(|err| {
        ToolCallError::InvalidParams(format!("log_path '{}' is not valid: {err}", log_path))
    })?;
    let global = crate::config::global::global_terrarium_dir()
        .canonicalize()
        .map_err(|err| ToolCallError::Execution(anyhow!("cannot resolve terrarium dir: {err}")))?;
    if !resolved.starts_with(global.join("projects")) {
        return Err(ToolCallError::InvalidParams(
            "log_path must be under ~/.terrarium/projects".to_string(),
        ));
    }
    if resolved
        .parent()
        .and_then(|parent| parent.file_name())
        .is_none_or(|name| name != "output")
    {
        return Err(ToolCallError::InvalidParams(
            "log_path must be inside a terrarium output directory".to_string(),
        ));
    }
    Ok(resolved)
}

fn tail_lines(text: &str, tail_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(tail_lines);
    lines[start..].join("\n")
}

pub(crate) fn command_timeout(
    timeout_secs: Option<u64>,
) -> Result<std::time::Duration, ToolCallError> {
    let Some(timeout_secs) = timeout_secs else {
        return Ok(LONG_COMMAND_TIMEOUT);
    };
    if timeout_secs == 0 {
        return Err(ToolCallError::InvalidParams(
            "timeout_secs must be greater than zero".to_string(),
        ));
    }
    if timeout_secs > MAX_COMMAND_TIMEOUT_SECS {
        return Err(ToolCallError::InvalidParams(format!(
            "timeout_secs must be <= {MAX_COMMAND_TIMEOUT_SECS}"
        )));
    }
    Ok(std::time::Duration::from_secs(timeout_secs))
}

pub(crate) fn validate_command_args(
    args: Vec<String>,
    label: &str,
) -> Result<Vec<String>, ToolCallError> {
    for arg in &args {
        if arg.is_empty() {
            return Err(ToolCallError::InvalidParams(format!(
                "{label} must not contain empty arguments"
            )));
        }
    }
    Ok(args)
}

fn compile_output_regex(
    pattern: Option<&str>,
    case_insensitive: bool,
) -> Result<Option<regex::Regex>, ToolCallError> {
    let Some(pattern) = pattern else {
        return Ok(None);
    };
    if pattern.is_empty() {
        return Err(ToolCallError::InvalidParams(
            "output_regex must not be empty".to_string(),
        ));
    }
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map(Some)
        .map_err(|err| ToolCallError::InvalidParams(format!("invalid output_regex: {err}")))
}

fn apply_output_regex(output: &str, regex: Option<&regex::Regex>) -> String {
    let Some(regex) = regex else {
        return output.to_string();
    };
    output
        .lines()
        .filter(|line| regex.is_match(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn display_command(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(program.to_string());
    parts.extend(args.iter().map(|arg| display_arg(arg)));
    parts.join(" ")
}

fn display_arg(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '='))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_output_regex_keeps_matching_lines() {
        let regex = compile_output_regex(Some("BitRoutes\\.balance|balanceResponse"), false)
            .unwrap()
            .unwrap();
        let output = "ignore\nBitRoutes.balance ok\nother\nbalanceResponse yes\n";

        assert_eq!(
            apply_output_regex(output, Some(&regex)),
            "BitRoutes.balance ok\nbalanceResponse yes"
        );
    }

    #[test]
    fn compile_output_regex_supports_case_insensitive_matching() {
        let regex = compile_output_regex(Some("bit "), true).unwrap().unwrap();

        assert_eq!(apply_output_regex("BIT ok\nmiss", Some(&regex)), "BIT ok");
    }

    fn scratch_project(label: &str) -> (PathBuf, PathBuf, crate::config::global::TestHomeGuard) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{label}_{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let guard = crate::config::global::override_home_for_tests(base.join("home"));
        (base, project, guard)
    }

    fn reported_log_path(result: &ToolCallResult) -> PathBuf {
        let text = &result.content[0].text;
        let start = text.find("log: `").expect("result names its log") + "log: `".len();
        let end = start + text[start..].find('`').unwrap();
        PathBuf::from(&text[start..end])
    }

    #[tokio::test]
    async fn commands_started_in_the_same_second_each_get_their_own_log() {
        let (base, project, _home) = scratch_project("same_second_logs");
        let timeout = std::time::Duration::from_secs(10);

        let mut logs = Vec::new();
        for _ in 0..3 {
            let result = run_external_command(&project, "true", &[], None, false, timeout, false)
                .await
                .expect("a second run must not fail on an existing log");
            logs.push(reported_log_path(&result));
        }

        logs.sort();
        logs.dedup();
        assert_eq!(logs.len(), 3, "runs shared a log: {logs:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn spilled_filtered_output_leaves_the_command_log_whole() {
        let (base, project, _home) = scratch_project("spill_keeps_log");
        let args = vec![
            "-c".to_string(),
            "i=0; while [ $i -lt 2000 ]; do echo line-$i; i=$((i+1)); done".to_string(),
        ];

        let result = run_external_command(
            &project,
            "sh",
            &args,
            Some("line-1"),
            false,
            std::time::Duration::from_secs(10),
            false,
        )
        .await
        .unwrap();

        let log = std::fs::read_to_string(reported_log_path(&result)).unwrap();
        assert!(
            log.starts_with("$ sh"),
            "the log lost its header: {log:.80}"
        );
        assert!(
            log.lines().any(|line| line == "line-0"),
            "the log lost output the regex filtered out"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn a_finished_background_command_does_not_linger_as_a_zombie() {
        let (base, project, _home) = scratch_project("background_reaped");

        let result = run_external_command(
            &project,
            "true",
            &[],
            None,
            false,
            std::time::Duration::from_secs(10),
            true,
        )
        .await
        .unwrap();
        let text = &result.content[0].text;
        let pid: i32 = text
            .lines()
            .find_map(|line| line.strip_prefix("pid: "))
            .expect("result names the pid")
            .parse()
            .unwrap();

        // A zombie still accepts signal 0; only a reaped process is gone.
        let gone = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err()
        });
        assert!(gone, "background process {pid} was never reaped");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn display_command_quotes_non_simple_args() {
        let args = vec![
            "compose".to_string(),
            "logs".to_string(),
            "BIT |balanceResponse".to_string(),
        ];

        assert_eq!(
            display_command("docker", &args),
            "docker compose logs 'BIT |balanceResponse'"
        );
    }
}
