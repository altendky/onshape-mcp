//! Tool definitions and dispatch logic.
//!
//! This module contains all tool metadata and pure business logic for tool execution.
//! Uses rmcp types directly to avoid unnecessary type conversions.

use std::sync::Arc;

use rmcp::{
    ErrorData,
    model::{CallToolResult, Content, ErrorCode, Tool, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::AuthStatusResult;

// ============================================================================
// Tool Registry
// ============================================================================

/// Returns all available tools.
#[must_use]
pub fn list_tools() -> Vec<Tool> {
    vec![tool_auth_status_def()]
}

/// Dispatches a tool call by name.
///
/// # Errors
///
/// Returns an error if the tool is not found or execution fails.
pub fn call_tool(
    name: &str,
    _arguments: Option<&Map<String, Value>>,
) -> Result<CallToolResult, ErrorData> {
    match name {
        "onshape_mcp_auth_status" => call_auth_status(),
        _ => Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("Unknown tool: {name}"),
            None,
        )),
    }
}

// ============================================================================
// Individual Tool Implementations
// ============================================================================

/// Empty input schema for tools with no parameters.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmptyInput {}

fn tool_auth_status_def() -> Tool {
    let schema = schemars::schema_for!(EmptyInput);
    let input_schema: Value = serde_json::to_value(schema).unwrap_or_default();
    let input_schema = input_schema.as_object().cloned().unwrap_or_default();

    Tool::new(
        "onshape_mcp_auth_status",
        "Returns authentication status (valid/invalid/expired/not_configured), \
         last check time, and a human-readable message",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

fn call_auth_status() -> Result<CallToolResult, ErrorData> {
    let result = AuthStatusResult::not_configured();
    let content = Content::json(&result)?;
    Ok(CallToolResult {
        content: vec![content],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn list_tools_includes_auth_status() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_mcp_auth_status"));
    }

    #[test]
    fn call_tool_auth_status_returns_not_configured() {
        let result = call_tool("onshape_mcp_auth_status", None).expect("should succeed");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);

        // Content is text containing JSON
        let content = &result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
    }

    #[test]
    fn call_tool_unknown_returns_not_found() {
        let err = call_tool("unknown_tool", None).expect_err("should fail");
        assert_eq!(err.code, ErrorCode::METHOD_NOT_FOUND);
        assert!(err.message.contains("unknown_tool"));
    }

    #[test]
    fn error_code_values() {
        // Verify rmcp's error codes match JSON-RPC spec
        assert_eq!(ErrorCode::METHOD_NOT_FOUND.0, -32601);
        assert_eq!(ErrorCode::INVALID_PARAMS.0, -32602);
        assert_eq!(ErrorCode::INTERNAL_ERROR.0, -32603);
    }
}
