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
use crate::config::{AuthConfig, CredentialStatus};

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
    arguments: Option<&Map<String, Value>>,
    auth_config: &AuthConfig,
) -> Result<CallToolResult, ErrorData> {
    match name {
        "onshape_mcp_auth_status" => {
            if let Some(args) = arguments
                && !args.is_empty()
            {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "onshape_mcp_auth_status expects no arguments",
                    None,
                ));
            }
            call_auth_status(auth_config)
        }
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

#[allow(clippy::expect_used)] // Schema generation for EmptyInput should never fail
fn tool_auth_status_def() -> Tool {
    let schema = schemars::schema_for!(EmptyInput);
    let input_schema: Value =
        serde_json::to_value(schema).expect("EmptyInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_mcp_auth_status",
        "Returns authentication status (valid/invalid/expired/not_configured), \
         last check time, and a human-readable message",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

fn call_auth_status(auth_config: &AuthConfig) -> Result<CallToolResult, ErrorData> {
    let result = match auth_config.credential_status() {
        CredentialStatus::NonePresent => AuthStatusResult::not_configured(),
        CredentialStatus::BothPresent => AuthStatusResult::not_validated(),
        CredentialStatus::Partial { missing } => AuthStatusResult::partial_credentials(missing),
    };
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
    use secrecy::SecretString;

    use super::*;

    fn no_creds() -> AuthConfig {
        AuthConfig::default()
    }

    fn both_creds() -> AuthConfig {
        AuthConfig {
            access_key: Some(SecretString::from("ak")),
            secret_key: Some(SecretString::from("sk")),
            ..AuthConfig::default()
        }
    }

    fn partial_creds_missing_secret() -> AuthConfig {
        AuthConfig {
            access_key: Some(SecretString::from("ak")),
            secret_key: None,
            ..AuthConfig::default()
        }
    }

    fn partial_creds_missing_access() -> AuthConfig {
        AuthConfig {
            access_key: None,
            secret_key: Some(SecretString::from("sk")),
            ..AuthConfig::default()
        }
    }

    #[test]
    fn list_tools_includes_auth_status() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_mcp_auth_status"));
    }

    #[test]
    fn call_tool_auth_status_returns_not_configured() {
        let config = no_creds();
        let result = call_tool("onshape_mcp_auth_status", None, &config).expect("should succeed");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);

        let content = &result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
    }

    #[test]
    fn call_tool_auth_status_returns_not_validated_with_creds() {
        let config = both_creds();
        let result = call_tool("onshape_mcp_auth_status", None, &config).expect("should succeed");
        assert_eq!(result.is_error, Some(false));

        let content = &result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_validated");
    }

    #[test]
    fn call_tool_auth_status_returns_partial_with_missing_key() {
        let config = partial_creds_missing_secret();
        let result = call_tool("onshape_mcp_auth_status", None, &config).expect("should succeed");
        assert_eq!(result.is_error, Some(false));

        let content = &result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert!(
            value["message"]
                .as_str()
                .is_some_and(|m| m.contains("secret_key"))
        );
    }

    #[test]
    fn call_tool_auth_status_returns_partial_with_missing_access_key() {
        let config = partial_creds_missing_access();
        let result = call_tool("onshape_mcp_auth_status", None, &config).expect("should succeed");
        assert_eq!(result.is_error, Some(false));

        let content = &result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert!(
            value["message"]
                .as_str()
                .is_some_and(|m| m.contains("access_key"))
        );
    }

    #[test]
    fn call_tool_unknown_returns_not_found() {
        let config = no_creds();
        let err = call_tool("unknown_tool", None, &config).expect_err("should fail");
        assert_eq!(err.code, ErrorCode::METHOD_NOT_FOUND);
        assert!(err.message.contains("unknown_tool"));
    }

    #[test]
    fn call_tool_auth_status_rejects_unexpected_arguments() {
        let config = no_creds();
        let mut args = Map::new();
        args.insert("unexpected".to_string(), Value::String("value".to_string()));
        let err =
            call_tool("onshape_mcp_auth_status", Some(&args), &config).expect_err("should fail");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn error_code_values() {
        // Verify rmcp's error codes match JSON-RPC spec
        assert_eq!(ErrorCode::METHOD_NOT_FOUND.0, -32601);
        assert_eq!(ErrorCode::INVALID_PARAMS.0, -32602);
        assert_eq!(ErrorCode::INTERNAL_ERROR.0, -32603);
    }
}
