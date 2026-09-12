//! Spawning a child process and collecting everything it wrote.

use super::types::ToolCallError;

pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) combined: String,
}

/// Spawns a fully-configured command and collects its output.
///
/// Every toolchain shares this tail; only the command construction differs
/// (cargo and swift just set a program and args, Gradle also picks the wrapper
/// and injects `JAVA_HOME`), so callers build the [`tokio::process::Command`]
/// and hand it over.
pub(crate) async fn spawn_capture(
    mut command: tokio::process::Command,
) -> Result<CommandOutput, ToolCallError> {
    let child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| ToolCallError::Execution(anyhow::anyhow!(err.to_string())))?;
    let output = child
        .wait_with_output()
        .await
        .map_err(|err| ToolCallError::Execution(anyhow::anyhow!(err.to_string())))?;
    Ok(command_output(
        output.stdout,
        output.stderr,
        output.status.success(),
    ))
}

pub(crate) fn command_output(stdout: Vec<u8>, stderr: Vec<u8>, success: bool) -> CommandOutput {
    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = String::from_utf8_lossy(&stderr).to_string();
    let combined = if stdout.is_empty() {
        stderr.clone()
    } else if stderr.is_empty() {
        stdout.clone()
    } else {
        format!("{stdout}\n{stderr}")
    };
    CommandOutput {
        success,
        stdout,
        stderr,
        combined,
    }
}
