//! Terrarium's MCP server: the build, git, and file tools a sandboxed Claude
//! session calls out to.
//!
//! The layers are transport ([`stdio`], [`http`]) → [`dispatch`] → [`tools`].
//! Only the transport differs between `terrarium mcp` and `terrarium run`;
//! everything below it is shared.

mod dispatch;
mod http;
pub mod output;
mod protocol;
mod stdio;
pub mod tools;

pub use http::serve_http;
pub use stdio::serve;

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::Result;

/// The tool surface a server instance serves, shared across all its requests.
#[derive(Clone)]
struct McpState {
    tools: tools::ProjectTools,
}

/// Builds the state for a root, as a workspace when one is configured there.
fn build_state(project_root: PathBuf) -> Result<Arc<McpState>> {
    if let Ok(ws_config) = crate::config::workspace::WorkspaceConfig::load(&project_root) {
        return Ok(Arc::new(McpState {
            tools: tools::ProjectTools::for_workspace(project_root, &ws_config),
        }));
    }
    Ok(Arc::new(McpState {
        tools: tools::ProjectTools::new(project_root),
    }))
}
