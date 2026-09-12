//! Recording a sandbox denial the agent hit, so the profile can be reviewed.

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::anyhow;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::tools::inputs::require_tool_args;
use crate::mcp::tools::paths::resolve_cwd;
use crate::mcp::tools::types::{ToolCallError, ToolCallResult, success_result};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportViolationInput {
    /// What operation was attempted and what error was encountered
    description: String,
    /// The command or operation that was denied, if applicable
    command: Option<String>,
    /// Optional working directory relative to project root
    #[serde(default)]
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

pub(crate) fn report_violation(
    project_root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = require_tool_args::<ReportViolationInput>(arguments)?;
    let effective_root = resolve_cwd(project_root, input.path.as_deref())?;
    let path = effective_root.join("violations.md");

    let needs_header = std::fs::metadata(&path)
        .map(|m| m.len() == 0)
        .unwrap_or(true);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| ToolCallError::Execution(anyhow!("failed to open violations.md: {err}")))?;

    let mut w = BufWriter::new(file);
    let write_err = |err: std::io::Error| {
        ToolCallError::Execution(anyhow!("failed to write violations.md: {err}"))
    };

    if needs_header {
        writeln!(w, "# Sandbox Violations\n").map_err(write_err)?;
    }
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    writeln!(w, "## {timestamp}\n").map_err(write_err)?;
    if let Some(cmd) = &input.command {
        writeln!(w, "**Command:** `{cmd}`\n").map_err(write_err)?;
    }
    writeln!(w, "{}\n", input.description).map_err(write_err)?;
    w.flush().map_err(write_err)?;

    Ok(success_result(format!(
        "Violation recorded in `{}`",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `path` selects a sub-project directory and `project` names it; both must be
    /// accepted, and `deny_unknown_fields` makes that worth pinning.
    #[test]
    fn report_violation_accepts_path_and_project() {
        let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("report_violation_path_project");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("svc-a")).unwrap();

        let result = report_violation(
            &base,
            Some(json!({
                "description": "sandbox denied /usr/bin/env",
                "command": "env",
                "path": "svc-a",
                "project": "svc-a"
            })),
        )
        .expect("path and project must both be accepted");

        assert!(result.content[0].text.contains("violations.md"));
        let written = std::fs::read_to_string(base.join("svc-a").join("violations.md")).unwrap();
        assert!(written.contains("# Sandbox Violations"));
        assert!(written.contains("sandbox denied /usr/bin/env"));
        assert!(written.contains("**Command:** `env`"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
