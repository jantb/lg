//! Turning a JSON-RPC message into an MCP response.
//!
//! Transport-independent: both [`super::stdio`] and [`super::http`] hand payloads
//! to [`process_payload`] and get back an optional response plus the protocol
//! version to report.
//!
//! One convention matters here. A tool that fails answers with a *successful*
//! JSON-RPC result carrying `isError: true`, not a JSON-RPC error — a failing
//! build is an answer, not a protocol fault, and clients surface the two very
//! differently.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::{Value, json};

use super::McpState;
use super::protocol::{jsonrpc_error, jsonrpc_success, resolve_protocol_version};
use super::tools;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    protocol_version: String,
}

#[derive(Debug, Deserialize)]
struct CallToolParams {
    name: String,
    arguments: Option<Value>,
}

/// Handles a single message or a JSON-RPC batch.
pub(super) async fn process_payload(
    state: &McpState,
    payload: Value,
    request_protocol: Option<&str>,
) -> (Option<Value>, Cow<'static, str>) {
    match payload {
        Value::Array(messages) => {
            let mut responses = Vec::new();
            let mut protocol = resolve_protocol_version(request_protocol, None);
            for message in messages {
                let (response, message_protocol) =
                    process_message(state, message, request_protocol).await;
                protocol = message_protocol;
                if let Some(response) = response {
                    responses.push(response);
                }
            }

            // A batch of pure notifications gets no response body at all.
            let payload = if responses.is_empty() {
                None
            } else {
                Some(Value::Array(responses))
            };

            (payload, protocol)
        }
        message => process_message(state, message, request_protocol).await,
    }
}

async fn process_message(
    state: &McpState,
    payload: Value,
    request_protocol: Option<&str>,
) -> (Option<Value>, Cow<'static, str>) {
    let Some(object) = payload.as_object() else {
        return (
            Some(jsonrpc_error(
                None,
                -32600,
                "JSON-RPC message must be an object".to_string(),
            )),
            resolve_protocol_version(request_protocol, None),
        );
    };

    let method = match object.get("method").and_then(Value::as_str) {
        Some(method) => method,
        None => {
            // Ignore client responses and unknown payloads that are not requests.
            return (None, resolve_protocol_version(request_protocol, None));
        }
    };

    let id = object.get("id").cloned();
    let params = object.get("params").cloned();
    let protocol = if method == "initialize" {
        match params
            .as_ref()
            .and_then(|value| serde_json::from_value::<InitializeParams>(value.clone()).ok())
        {
            Some(params) => {
                resolve_protocol_version(request_protocol, Some(&params.protocol_version))
            }
            None => resolve_protocol_version(request_protocol, None),
        }
    } else {
        resolve_protocol_version(request_protocol, None)
    };

    // No id means a notification: acted on, never answered.
    if id.is_none() {
        handle_notification(method);
        return (None, protocol);
    }

    let id = id.unwrap_or(Value::Null);
    let response = match method {
        "initialize" => handle_initialize(state, id, params),
        "ping" => jsonrpc_success(id, json!({})),
        "tools/list" => {
            let tools = state.tools.definitions();
            jsonrpc_success(id, json!({ "tools": tools }))
        }
        "tools/call" => handle_tool_call(state, id, params).await,
        _ => jsonrpc_error(Some(id), -32601, format!("method not found: {method}")),
    };

    (Some(response), protocol)
}

fn handle_initialize(state: &McpState, id: Value, params: Option<Value>) -> Value {
    let params = match params {
        Some(params) => params,
        None => return jsonrpc_error(Some(id), -32602, "missing initialize params".to_string()),
    };

    let params = match serde_json::from_value::<InitializeParams>(params) {
        Ok(params) => params,
        Err(err) => {
            return jsonrpc_error(
                Some(id),
                -32602,
                format!("invalid initialize params: {err}"),
            );
        }
    };

    let protocol_version = resolve_protocol_version(None, Some(&params.protocol_version));
    jsonrpc_success(
        id,
        json!({
            "protocolVersion": protocol_version,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": state.tools.server_name(),
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": state.tools.instructions()
        }),
    )
}

/// Every outcome here is a `jsonrpc_success`; failures are carried as an
/// `isError` result so a broken build is not reported as a broken protocol.
async fn handle_tool_call(state: &McpState, id: Value, params: Option<Value>) -> Value {
    let params = match params {
        Some(params) => params,
        None => {
            return jsonrpc_success(
                id,
                json!(tool_error_result("missing tools/call params".to_string())),
            );
        }
    };

    let call = match serde_json::from_value::<CallToolParams>(params) {
        Ok(call) => call,
        Err(err) => {
            return jsonrpc_success(
                id,
                json!(tool_error_result(format!(
                    "invalid tools/call params: {err}"
                ))),
            );
        }
    };

    match state.tools.call(&call.name, call.arguments).await {
        Ok(result) => jsonrpc_success(id, json!(result)),
        Err(tools::ToolCallError::InvalidParams(message))
        | Err(tools::ToolCallError::UnknownTool(message)) => {
            jsonrpc_success(id, json!(tool_error_result(message)))
        }
        Err(tools::ToolCallError::Execution(err)) => {
            jsonrpc_success(id, json!(tool_error_result(err.to_string())))
        }
    }
}

/// Notifications carry no reply. The known ones need no action; unknown ones are
/// ignored rather than rejected, as the spec requires.
fn handle_notification(method: &str) {
    match method {
        "notifications/initialized" => {}
        "notifications/cancelled" => {}
        _ => {}
    }
}

fn tool_error_result(message: String) -> tools::ToolCallResult {
    tools::ToolCallResult {
        content: vec![tools::TextContent {
            kind: "text",
            text: message,
        }],
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::ToolMode;
    use std::path::PathBuf;

    fn state_for(mode: ToolMode) -> McpState {
        McpState {
            tools: tools::ProjectTools::for_mode(mode, PathBuf::from(".")),
        }
    }

    /// Names every tool in a `tools/list` response.
    async fn listed_tools(state: &McpState) -> Vec<String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/list",
            "params": {}
        });
        let (response, _) = process_payload(state, payload, None).await;
        response.unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn initialize_declares_tools_capability() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26"
            }
        });

        let (response, protocol) =
            process_payload(&state_for(ToolMode::Cargo), payload, None).await;
        let response = response.unwrap();

        assert_eq!(protocol, "2025-03-26");
        assert_eq!(response["result"]["capabilities"]["tools"], json!({}));
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "terrarium-cargo-tools"
        );
        assert!(
            response["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("git inspection tools")
        );
    }

    #[tokio::test]
    async fn tools_list_returns_cargo_tools() {
        let names = listed_tools(&state_for(ToolMode::Cargo)).await;
        for expected in [
            "cargo_check",
            "cargo_test",
            "cargo_update",
            "cargo_upgrade_incompatible",
            "git_status",
            "git_unmerged",
            "git_log",
        ] {
            assert!(names.iter().any(|n| n == expected), "missing {expected}");
        }
    }

    #[tokio::test]
    async fn tools_list_returns_gradle_tools_for_gradle_mode() {
        let names = listed_tools(&state_for(ToolMode::Gradle)).await;
        for expected in [
            "gradle_check",
            "gradle_test",
            "git_status",
            "git_unmerged",
            "git_show",
        ] {
            assert!(names.iter().any(|n| n == expected), "missing {expected}");
        }
    }

    #[tokio::test]
    async fn invalid_tool_call_params_return_tool_error_result() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "cargo_build",
                "arguments": {
                    "release": "nope"
                }
            }
        });

        let (response, _) = process_payload(&state_for(ToolMode::Cargo), payload, None).await;
        let response = response.unwrap();

        assert_eq!(response["result"]["isError"], json!(true));
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("invalid tool arguments")
        );
    }
}
