//! Passthroughs to the project's own build runners: make, just, and docker.
//!
//! These deliberately take structured argv rather than a command string — there
//! is no shell in the sandbox to interpret pipes or quoting, so `output_regex`
//! replaces `| grep` and `args` replaces word splitting.

use std::path::Path;

use anyhow::anyhow;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::tools::inputs::{parse_tool_args, require_tool_args};
use crate::mcp::tools::paths::resolve_cwd;
use crate::mcp::tools::types::{ToolCallError, ToolCallResult};

use super::command::{
    LONG_COMMAND_TIMEOUT, command_timeout, run_external_command, validate_command_args,
};

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MakeRunInput {
    /// Optional make target. Omit to run make's default target.
    target: Option<String>,
    /// Additional argv passed directly to make before the optional target, such
    /// as ["-j4", "VAR=value"].
    args: Vec<String>,
    /// Optional Rust regex. When provided, only matching output lines are
    /// returned.
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    case_insensitive: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Start the command and return immediately with a live log path and PID.
    background: Option<bool>,
    /// Optional working directory relative to project root
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct JustRunInput {
    /// Optional just recipe. Omit to run just's default recipe.
    recipe: Option<String>,
    /// Additional argv passed directly to just before the optional recipe, such
    /// as ["--set", "var", "value"] or recipe arguments.
    args: Vec<String>,
    /// Optional Rust regex. When provided, only matching output lines are
    /// returned.
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    case_insensitive: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Start the command and return immediately with a live log path and PID.
    background: Option<bool>,
    /// Optional working directory relative to project root
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DockerRunInput {
    /// Arguments passed directly after docker. Example: ["compose", "logs",
    /// "gravty-mock", "--tail", "1000"].
    args: Vec<String>,
    /// Optional Rust regex. When provided, only matching output lines are
    /// returned.
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    #[serde(default)]
    case_insensitive: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[serde(default)]
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Start the command and return immediately with a live log path and PID.
    #[serde(default)]
    background: Option<bool>,
    /// Optional working directory relative to project root
    #[serde(default)]
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

pub(crate) async fn make_install(project_root: &Path) -> Result<ToolCallResult, ToolCallError> {
    let makefile = project_root.join("Makefile");
    if !makefile.exists() {
        return Err(ToolCallError::Execution(anyhow!(
            "no Makefile found in '{}'",
            project_root.display()
        )));
    }
    run_external_command(
        project_root,
        "make",
        &["install".to_string()],
        None,
        false,
        LONG_COMMAND_TIMEOUT,
        false,
    )
    .await
}

pub(crate) async fn make_run(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = parse_tool_args::<MakeRunInput>(arguments)?;
    let effective_root = resolve_cwd(project_root, input.path.as_deref())?;
    let mut args = validate_command_args(input.args, "make args")?;
    if let Some(target) = input.target {
        args.push(non_empty(&target, "make target")?);
    }
    run_external_command(
        &effective_root,
        "make",
        &args,
        input.output_regex.as_deref(),
        input.case_insensitive.unwrap_or(false),
        command_timeout(input.timeout_secs)?,
        input.background.unwrap_or(false),
    )
    .await
}

pub(crate) async fn just_run(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = parse_tool_args::<JustRunInput>(arguments)?;
    let effective_root = resolve_cwd(project_root, input.path.as_deref())?;
    let mut args = validate_command_args(input.args, "just args")?;
    if let Some(recipe) = input.recipe {
        args.push(non_empty(&recipe, "just recipe")?);
    }
    run_external_command(
        &effective_root,
        "just",
        &args,
        input.output_regex.as_deref(),
        input.case_insensitive.unwrap_or(false),
        command_timeout(input.timeout_secs)?,
        input.background.unwrap_or(false),
    )
    .await
}

pub(crate) async fn docker_run(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = require_tool_args::<DockerRunInput>(arguments)?;
    let effective_root = resolve_cwd(project_root, input.path.as_deref())?;
    let args = validate_command_args(input.args, "docker args")?;
    if args.is_empty() {
        return Err(ToolCallError::InvalidParams(
            "docker args must contain at least one argument".to_string(),
        ));
    }
    run_external_command(
        &effective_root,
        "docker",
        &args,
        input.output_regex.as_deref(),
        input.case_insensitive.unwrap_or(false),
        command_timeout(input.timeout_secs)?,
        input.background.unwrap_or(false),
    )
    .await
}

/// A blank target or recipe would silently run the runner's default instead of
/// what the caller asked for, so it is rejected rather than trimmed away.
fn non_empty(value: &str, label: &str) -> Result<String, ToolCallError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}
