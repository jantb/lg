//! The wire types an MCP tool call is expressed in: what a tool advertises,
//! what it answers with, and how it fails.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Serialize)]
pub struct ToolCallResult {
    pub content: Vec<TextContent>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug)]
pub enum ToolCallError {
    UnknownTool(String),
    InvalidParams(String),
    Execution(anyhow::Error),
}

pub(crate) fn success_result(text: String) -> ToolCallResult {
    ToolCallResult {
        content: vec![TextContent { kind: "text", text }],
        is_error: false,
    }
}
