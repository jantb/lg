//! JSON-RPC envelopes and MCP protocol-version negotiation.

use std::borrow::Cow;

use serde_json::{Value, json};

pub(super) const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";
const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Every revision this server speaks, newest first.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    LATEST_PROTOCOL_VERSION,
    "2025-06-18",
    DEFAULT_PROTOCOL_VERSION,
    "2024-11-05",
];

/// Picks the revision to answer in. A version named in `initialize` wins over the
/// transport header; anything unrecognized falls back to the newest supported
/// revision rather than failing the connection.
pub(super) fn resolve_protocol_version(
    request_protocol: Option<&str>,
    initialize_protocol: Option<&str>,
) -> Cow<'static, str> {
    let requested = initialize_protocol
        .or(request_protocol)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    match SUPPORTED_PROTOCOL_VERSIONS
        .iter()
        .find(|&&v| v == requested)
    {
        Some(&v) => Cow::Borrowed(v),
        None => Cow::Borrowed(LATEST_PROTOCOL_VERSION),
    }
}

pub(super) fn jsonrpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

pub(super) fn jsonrpc_error(id: Option<Value>, code: i32, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_prefers_the_version_named_in_initialize() {
        assert_eq!(
            resolve_protocol_version(Some("2024-11-05"), Some("2025-06-18")),
            "2025-06-18"
        );
    }

    #[test]
    fn negotiation_falls_back_to_the_transport_header() {
        assert_eq!(
            resolve_protocol_version(Some("2024-11-05"), None),
            "2024-11-05"
        );
    }

    #[test]
    fn negotiation_defaults_when_nothing_is_stated() {
        assert_eq!(
            resolve_protocol_version(None, None),
            DEFAULT_PROTOCOL_VERSION
        );
    }

    /// An unknown revision must not drop the connection — the newest supported
    /// one is the best guess for a client newer than this server.
    #[test]
    fn negotiation_answers_unknown_versions_with_the_latest() {
        assert_eq!(
            resolve_protocol_version(None, Some("1999-01-01")),
            LATEST_PROTOCOL_VERSION
        );
    }
}
