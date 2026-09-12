//! Line-delimited JSON-RPC over stdin/stdout.
//!
//! Used by `terrarium mcp`, where the client launches the server as a child
//! process. `terrarium run` uses [`super::http`] instead, so the sandboxed
//! session can reach a server started outside it.

use std::path::PathBuf;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::error::{Result, TerrariumError};

use super::build_state;
use super::dispatch::process_payload;
use super::protocol::jsonrpc_error;

/// Start the MCP stdio server for the given project root.
pub async fn serve(project_root: PathBuf) -> Result<()> {
    let state = build_state(project_root)?;
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(value) => process_payload(state.as_ref(), value, None).await.0,
            Err(err) => Some(jsonrpc_error(
                None,
                -32700,
                format!("failed to parse JSON-RPC payload: {err}"),
            )),
        };

        if let Some(response) = response {
            let encoded =
                serde_json::to_vec(&response).map_err(|err| TerrariumError::SandboxExecFailed {
                    reason: format!("failed to encode MCP stdio response: {err}"),
                })?;
            writer.write_all(&encoded).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }

    Ok(())
}
