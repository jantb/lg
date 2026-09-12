//! Standing up the per-run MCP server the sandboxed session talks to.
//!
//! The server runs in *this* process, outside the sandbox, which is the whole
//! point: builds and git run unrestricted while the session that requests them
//! does not. It binds a fresh port each run, so the registration in `.mcp.json`
//! is rewritten every time and removed when the run ends.

use std::path::Path;

use crate::config::workspace::WorkspaceConfig;
use crate::error::{Result, TerrariumError};

/// Starts the server, waits for it to accept connections, and registers it with
/// Claude — including the `CLAUDE.local.md` guidance for the tools it provides.
pub(super) async fn start_and_register(
    project_root: &Path,
    ws_config: Option<&WorkspaceConfig>,
) -> Result<()> {
    // Claude prompts for confirmation the first time bypass mode is used;
    // record the acceptance so the launch flags apply without interaction.
    if let Err(err) = crate::app::ensure_bypass_permissions_accepted() {
        eprintln!("terrarium: warning: could not pre-accept bypass permissions mode: {err}");
    }
    // Configure CLAUDE.local.md - workspace or single-project
    if let Some(ws) = ws_config {
        crate::app::configure_workspace(project_root, &ws.projects)?;
    } else {
        crate::app::configure_claude(project_root, false)?;
    }

    let port = find_free_port()?;
    let root_clone = project_root.to_path_buf();
    println!("terrarium: starting built-in MCP server on port {port}");
    tokio::spawn(async move {
        if let Err(err) = crate::mcp::serve_http(root_clone, port).await {
            eprintln!("terrarium: built-in MCP server on port {port} failed: {err}");
        }
    });
    wait_for_http_ready(port).await?;
    println!("terrarium: built-in MCP server is accepting connections on port {port}");

    crate::app::register_mcp_server_http(project_root, port)?;
    // Read the entry back: a stale or mismatched registration would otherwise
    // only surface as a puzzling missing-tools session.
    let registered_url = crate::app::verify_registered_mcp_server_http(project_root, port)?;
    println!("terrarium: verified local Claude MCP registration at {registered_url}");
    Ok(())
}

/// Removes the registration, ignoring failures — the run is already over.
pub(super) fn unregister(project_root: &Path) {
    let _ = crate::app::unregister_mcp_server_http(project_root);
}

/// Binds a `TcpListener` to `127.0.0.1:0` and returns the OS-assigned port.
fn find_free_port() -> Result<u16> {
    use std::net::TcpListener;
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| TerrariumError::SandboxExecFailed {
            reason: format!("failed to find free port: {e}"),
        })?;
    let port = listener
        .local_addr()
        .map_err(|e| TerrariumError::SandboxExecFailed {
            reason: format!("failed to read local addr: {e}"),
        })?
        .port();
    Ok(port)
}

/// Polls `127.0.0.1:<port>` until a TCP connection succeeds or ~2 s elapse.
async fn wait_for_http_ready(port: u16) -> Result<()> {
    use std::time::Duration;
    for _ in 0..20u8 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(TerrariumError::SandboxExecFailed {
        reason: format!(
            "built-in MCP server did not start listening on 127.0.0.1:{port} within 2 seconds"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_free_port_returns_nonzero_port() {
        let port = find_free_port().unwrap();
        assert!(port > 0, "expected a non-zero port, got {port}");
    }
}
