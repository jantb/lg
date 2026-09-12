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
use crate::mcp::tools::report::{MAX_OUTPUT_CHARS, format_output_block, truncate_text};
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

    let child = StdCommand::new(program)
        .args(args)
        .current_dir(project_root)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;
    let pid = child.id();
    drop(child);

    Ok(success_result(format!(
        "{display_command} started in background\npid: {pid}\nlog: `{}`",
        log_path.display()
    )))
}

fn command_output_path(project_root: &Path, label: &str) -> Result<PathBuf, ToolCallError> {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    Ok(command_output_dir(project_root)?.join(format!("{label}-{ts}.log")))
}

fn command_output_dir(project_root: &Path) -> Result<PathBuf, ToolCallError> {
    let dir = crate::config::project::global_profile_dir(project_root).join("output");
    crate::config::project::create_private_dir(&dir).map_err(|err| {
        ToolCallError::Execution(anyhow!("failed to create command output directory: {err}"))
    })?;
    Ok(dir)
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
