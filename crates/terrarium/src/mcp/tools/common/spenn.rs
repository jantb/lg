//! Passthrough to `spenn-api-client`, a project-specific scenario runner.
//!
//! Not part of terrarium's core surface — it is exposed because the sandbox
//! blocks running it from a shell.

use std::path::Path;
use std::process::Stdio;

use anyhow::anyhow;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::process::Command;

use crate::mcp::tools::inputs::{parse_tool_args, require_tool_args};
use crate::mcp::tools::process::command_output;
use crate::mcp::tools::report::format_output_block;
use crate::mcp::tools::types::{ToolCallError, ToolCallResult, success_result};

const SPENN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpennRunScenarioInput {
    /// Scenario name to run
    name: String,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SpennListScenariosInput {
    /// Project name to target when multiple projects are available
    #[allow(dead_code)]
    project: Option<String>,
}

pub(crate) async fn spenn_list_scenarios(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let _input = parse_tool_args::<SpennListScenariosInput>(arguments)?;
    run_spenn_api_client(project_root, &["list-scenarios"]).await
}

pub(crate) async fn spenn_run_scenario(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = require_tool_args::<SpennRunScenarioInput>(arguments)?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ToolCallError::InvalidParams(
            "scenario name must not be empty".to_string(),
        ));
    }
    run_spenn_api_client(project_root, &["run-scenario", name]).await
}

async fn run_spenn_api_client(
    project_root: &Path,
    args: &[&str],
) -> Result<ToolCallResult, ToolCallError> {
    let mut command = Command::new("spenn-api-client");
    command
        .args(args)
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = command
        .spawn()
        .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;
    let output = tokio::time::timeout(SPENN_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            ToolCallError::Execution(anyhow!("spenn-api-client timed out after 5 minutes"))
        })?
        .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;

    let out = command_output(output.stdout, output.stderr, output.status.success());
    Ok(success_result(if out.success {
        if out.combined.is_empty() {
            "spenn-api-client completed".to_string()
        } else {
            out.combined
        }
    } else {
        format!(
            "spenn-api-client **failed**{}",
            format_output_block(project_root, "spenn-api-client", &out.combined)
        )
    }))
}
