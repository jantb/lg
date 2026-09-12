//! Streamable HTTP transport, bound to loopback only.
//!
//! `terrarium run` starts this before entering the sandbox and registers its port
//! in the project's `.mcp.json`, so the sandboxed Claude session reaches build
//! tools that run outside the sandbox. Since any local process could reach the
//! port, requests carrying a non-loopback `Origin` are refused.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::error::{Result, TerrariumError};

use super::dispatch::process_payload;
use super::protocol::{DEFAULT_PROTOCOL_VERSION, jsonrpc_error};
use super::{McpState, build_state};

const PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";

/// Start an MCP HTTP server on `127.0.0.1:<port>` for the given project root.
pub async fn serve_http(project_root: PathBuf, port: u16) -> Result<()> {
    let addr: SocketAddr =
        format!("127.0.0.1:{port}")
            .parse()
            .map_err(|e| TerrariumError::SandboxExecFailed {
                reason: format!("invalid bind address: {e}"),
            })?;

    let state = build_state(project_root)?;
    let app = Router::new()
        .route("/mcp", post(handle_http_post).get(handle_http_get))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        TerrariumError::SandboxExecFailed {
            reason: format!("MCP HTTP server bind failed: {e}"),
        }
    })?;

    axum::serve(listener, app)
        .await
        .map_err(|e| TerrariumError::SandboxExecFailed {
            reason: format!("MCP HTTP server error: {e}"),
        })?;

    Ok(())
}

async fn handle_http_get() -> Response {
    // Streamable HTTP allows GET to return 405 when the server does not provide SSE streams.
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

async fn handle_http_post(
    State(state): State<Arc<McpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "forbidden origin").into_response();
    }

    let request_protocol = headers
        .get(PROTOCOL_VERSION_HEADER)
        .and_then(|value| value.to_str().ok());

    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(payload) => payload,
        Err(err) => {
            return json_response(
                jsonrpc_error(
                    None,
                    -32700,
                    format!("failed to parse JSON-RPC payload: {err}"),
                ),
                request_protocol.unwrap_or(DEFAULT_PROTOCOL_VERSION),
            );
        }
    };

    let (response, protocol_version) =
        process_payload(state.as_ref(), payload, request_protocol).await;
    match response {
        Some(response) => json_response(response, &protocol_version),
        // Notification-only payloads have nothing to answer with.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn json_response(payload: Value, protocol_version: &str) -> Response {
    let mut response = Json(payload).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(protocol_version) {
        response
            .headers_mut()
            .insert(PROTOCOL_VERSION_HEADER, value);
    }
    response
}

/// Guards against DNS-rebinding from a browser: a request with no `Origin` is a
/// direct client, anything else must name loopback.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };

    matches!(
        origin,
        "null"
            | "http://localhost"
            | "https://localhost"
            | "http://127.0.0.1"
            | "https://127.0.0.1"
    ) || origin.starts_with("http://localhost:")
        || origin.starts_with("https://localhost:")
        || origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("https://127.0.0.1:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_local_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("https://example.com"));
        assert!(!origin_allowed(&headers));
    }

    #[test]
    fn accepts_loopback_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:3000"));
        assert!(origin_allowed(&headers));
    }

    #[tokio::test]
    async fn get_transport_returns_method_not_allowed() {
        let response = handle_http_get().await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
