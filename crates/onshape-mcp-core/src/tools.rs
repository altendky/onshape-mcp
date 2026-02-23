//! Tool definitions and dispatch logic.
//!
//! This module contains all tool metadata and pure business logic for tool execution.
//! Uses rmcp types directly to avoid unnecessary type conversions.
//!
//! ## Effect Pattern
//!
//! Tool dispatch returns a [`ToolResult`] which is either:
//! - `Immediate` — the tool completed with no I/O needed
//! - `OnshapeApiRequest` — the tool needs an HTTP request executed by the I/O layer

use std::collections::HashMap;
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
use crate::openapi::{ApiRequest, OpenApiSpec, SearchFilters};

// ============================================================================
// Effect Type
// ============================================================================

/// Result of dispatching a tool call.
///
/// Tools that need no I/O return `Immediate`. Tools that need an HTTP request
/// to the Onshape API return `OnshapeApiRequest`, which the I/O layer must
/// execute and then pass back through [`process_api_response`].
pub enum ToolResult {
    /// Tool completed immediately with no I/O needed.
    Immediate(Result<CallToolResult, ErrorData>),
    /// Tool needs an HTTP request to the Onshape API.
    OnshapeApiRequest {
        /// The HTTP request to execute.
        request: ApiRequest,
    },
}

/// Convert a raw HTTP response from the Onshape API into a [`CallToolResult`].
///
/// # Arguments
///
/// * `status` - HTTP status code
/// * `body` - Response body as a string
///
/// # Errors
///
/// Returns an error if the response cannot be processed.
pub fn process_api_response(status: u16, body: &str) -> Result<CallToolResult, ErrorData> {
    let is_success = (200..300).contains(&status);

    if is_success {
        // Try to parse as JSON for nice formatting
        let content = if let Ok(json_val) = serde_json::from_str::<Value>(body) {
            Content::json(&json_val).map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("failed to serialize API response: {e}"),
                    None,
                )
            })?
        } else {
            Content::text(body)
        };

        Ok(CallToolResult {
            content: vec![content],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        })
    } else {
        let content = Content::text(format!("API error (HTTP {status}): {body}"));
        Ok(CallToolResult {
            content: vec![content],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        })
    }
}

// ============================================================================
// Tool Registry
// ============================================================================

/// Returns all available tools.
#[must_use]
pub fn list_tools() -> Vec<Tool> {
    vec![
        tool_auth_status_def(),
        tool_api_search_def(),
        tool_api_explain_def(),
        tool_api_call_def(),
    ]
}

/// Dispatches a tool call by name.
///
/// # Errors
///
/// Returns an error (via `ToolResult::Immediate`) if the tool is not found
/// or input validation fails. Returns `ToolResult::OnshapeApiRequest` if the
/// tool needs an HTTP request executed.
#[must_use]
pub fn call_tool(
    name: &str,
    arguments: Option<&Map<String, Value>>,
    auth_config: &AuthConfig,
    spec: Option<&OpenApiSpec>,
) -> ToolResult {
    match name {
        "onshape_mcp_auth_status" => ToolResult::Immediate(call_auth_status(auth_config)),
        "onshape_api_search" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolResult::Immediate(Err(e)),
            };
            ToolResult::Immediate(call_api_search(arguments, spec))
        }
        "onshape_api_explain" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolResult::Immediate(Err(e)),
            };
            ToolResult::Immediate(call_api_explain(arguments, spec))
        }
        "onshape_api_call" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolResult::Immediate(Err(e)),
            };
            call_api_call(arguments, spec)
        }
        _ => ToolResult::Immediate(Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("Unknown tool: {name}"),
            None,
        ))),
    }
}

// ============================================================================
// Input Schemas
// ============================================================================

/// Empty input schema for tools with no parameters.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmptyInput {}

/// Input schema for `onshape_api_search`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ApiSearchInput {
    /// Free-text search query. Matches against endpoint names, paths,
    /// descriptions, and tags. Leave empty to list all endpoints.
    pub query: String,
    /// Filter by HTTP method (e.g., "GET", "POST", "DELETE").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Filter by tag name (e.g., "Document", "Assembly", "`PartStudio`").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// Input schema for `onshape_api_explain`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ApiExplainInput {
    /// The operation ID of the endpoint to explain (from search results).
    pub endpoint: String,
}

/// Input schema for `onshape_api_call`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ApiCallInput {
    /// The operation ID of the endpoint to call.
    pub endpoint: String,
    /// Path parameters (e.g., `{"did": "abc123", "wid": "def456"}`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub path_params: HashMap<String, String>,
    /// Query parameters (e.g., `{"q": "robot arm", "limit": "10"}`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub query_params: HashMap<String, String>,
    /// JSON string for the request body (for POST/PUT/PATCH endpoints).
    /// Use `onshape_api_explain` to see the expected schema for each endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

// ============================================================================
// Tool Definitions
// ============================================================================

#[allow(clippy::expect_used)] // Schema generation should never fail
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

#[allow(clippy::expect_used)]
fn tool_api_search_def() -> Tool {
    let schema = schemars::schema_for!(ApiSearchInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("ApiSearchInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_api_search",
        "Find Onshape API endpoints by keyword or filter. Returns brief summaries \
         (endpoint ID, method, path template, one-line description). Use this to \
         discover available endpoints before calling onshape_api_explain for details.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

#[allow(clippy::expect_used)]
fn tool_api_explain_def() -> Tool {
    let schema = schemars::schema_for!(ApiExplainInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("ApiExplainInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_api_explain",
        "Get full details for a specific Onshape API endpoint. Returns parameter schemas, \
         types, required/optional flags, request/response schemas. Use the endpoint's \
         operationId from onshape_api_search results.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

#[allow(clippy::expect_used)]
fn tool_api_call_def() -> Tool {
    let schema = schemars::schema_for!(ApiCallInput);
    let input_schema: Value =
        serde_json::to_value(schema).expect("ApiCallInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_api_call",
        "Invoke an Onshape API endpoint. Provide the operationId and structured parameters \
         (path_params, query_params, body). Path parameters are named fields (e.g., \
         {\"did\": \"abc123\"}), not baked into a URL string. Returns the API response.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(false).destructive(true))
}

// ============================================================================
// Tool Implementations
// ============================================================================

fn call_auth_status(auth_config: &AuthConfig) -> Result<CallToolResult, ErrorData> {
    let method = auth_config.method;
    let result = match auth_config.credential_status() {
        CredentialStatus::NonePresent => AuthStatusResult::not_configured(method),
        CredentialStatus::BothPresent => AuthStatusResult::not_validated(method),
        CredentialStatus::Partial { missing } | CredentialStatus::OAuthPartial { missing } => {
            AuthStatusResult::partial_credentials(missing, method)
        }
        CredentialStatus::OAuthConfigured => {
            // OAuth client credentials are configured, but we don't know about
            // tokens here (that's the I/O layer's responsibility).
            // Report that OAuth is configured but no token status is available.
            AuthStatusResult::oauth_not_configured()
        }
    };
    let content = Content::json(&result)?;
    Ok(CallToolResult {
        content: vec![content],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    })
}

fn call_api_search(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiSearchInput = parse_arguments(arguments)?;
    let filters = SearchFilters {
        method: input.method,
        tag: input.tag,
    };
    let results = spec.search(&input.query, &filters);

    let content = Content::json(&results).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize search results: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult {
        content: vec![content],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    })
}

fn call_api_explain(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiExplainInput = parse_arguments(arguments)?;
    let detail = spec
        .explain(&input.endpoint)
        .map_err(|e| ErrorData::new(ErrorCode::INVALID_PARAMS, format!("{e}"), None))?;

    let content = Content::json(&detail).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize endpoint detail: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult {
        content: vec![content],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    })
}

fn call_api_call(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolResult {
    let input: ApiCallInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return ToolResult::Immediate(Err(e)),
    };

    let body: Option<Value> = match input.body.as_deref().map(serde_json::from_str).transpose() {
        Ok(v) => v,
        Err(e) => {
            return ToolResult::Immediate(Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("invalid body JSON: {e}"),
                None,
            )));
        }
    };

    if body == Some(Value::Null) {
        return ToolResult::Immediate(Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "body parsed as JSON null; omit the body field instead of passing \"null\"".to_string(),
            None,
        )));
    }

    let request = match spec.build_request(
        &input.endpoint,
        &input.path_params,
        &input.query_params,
        body,
    ) {
        Ok(req) => req,
        Err(e) => {
            return ToolResult::Immediate(Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("{e}"),
                None,
            )));
        }
    };

    ToolResult::OnshapeApiRequest { request }
}

/// Unwrap the `OpenAPI` spec reference or return an internal error.
fn require_spec(spec: Option<&OpenApiSpec>) -> Result<&OpenApiSpec, ErrorData> {
    spec.ok_or_else(|| ErrorData::new(ErrorCode::INTERNAL_ERROR, "OpenAPI spec not loaded", None))
}

/// Parse tool arguments from the MCP request into a typed struct.
fn parse_arguments<T: serde::de::DeserializeOwned>(
    arguments: Option<&Map<String, Value>>,
) -> Result<T, ErrorData> {
    let args_value =
        arguments.map_or_else(|| Value::Object(Map::new()), |m| Value::Object(m.clone()));

    serde_json::from_value(args_value).map_err(|e| {
        ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!("invalid arguments: {e}"),
            None,
        )
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
    use crate::openapi::HttpMethod;

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

    fn test_spec() -> OpenApiSpec {
        OpenApiSpec::from_json(
            r#"{
                "openapi": "3.0.1",
                "info": { "title": "Test API", "version": "1.0" },
                "servers": [{ "url": "https://example.com/api/v1" }],
                "paths": {
                    "/documents": {
                        "get": {
                            "operationId": "getDocuments",
                            "summary": "List documents",
                            "tags": ["Document"],
                            "parameters": [
                                {
                                    "name": "q",
                                    "in": "query",
                                    "required": false,
                                    "schema": { "type": "string" },
                                    "description": "Search query"
                                }
                            ],
                            "responses": { "200": {} }
                        }
                    },
                    "/documents/{did}": {
                        "get": {
                            "operationId": "getDocument",
                            "summary": "Get document by ID",
                            "tags": ["Document"],
                            "parameters": [
                                {
                                    "name": "did",
                                    "in": "path",
                                    "required": true,
                                    "schema": { "type": "string" },
                                    "description": "Document ID"
                                }
                            ],
                            "responses": { "200": {} }
                        }
                    },
                    "/documents/search": {
                        "post": {
                            "operationId": "searchDocuments",
                            "summary": "Search documents",
                            "tags": ["Document"],
                            "requestBody": {
                                "content": {
                                    "application/json;charset=UTF-8; qs=0.09": {
                                        "schema": {
                                            "type": "object",
                                            "properties": {
                                                "rawQuery": { "type": "string" },
                                                "limit": { "type": "integer" }
                                            }
                                        }
                                    }
                                },
                                "required": true
                            },
                            "responses": { "200": {} }
                        }
                    }
                },
                "components": { "schemas": {} }
            }"#,
        )
        .expect("test spec should parse")
    }

    fn assert_immediate_ok(result: ToolResult) -> CallToolResult {
        match result {
            ToolResult::Immediate(Ok(r)) => r,
            ToolResult::Immediate(Err(e)) => panic!("expected Ok, got Err: {e:?}"),
            ToolResult::OnshapeApiRequest { .. } => panic!("expected Immediate, got ApiRequest"),
        }
    }

    fn assert_immediate_err(result: ToolResult) -> ErrorData {
        match result {
            ToolResult::Immediate(Err(e)) => e,
            ToolResult::Immediate(Ok(_)) => panic!("expected Err, got Ok"),
            ToolResult::OnshapeApiRequest { .. } => panic!("expected Immediate, got ApiRequest"),
        }
    }

    fn assert_api_request(result: ToolResult) -> ApiRequest {
        match result {
            ToolResult::OnshapeApiRequest { request } => request,
            ToolResult::Immediate(Ok(_)) => panic!("expected ApiRequest, got Immediate Ok"),
            ToolResult::Immediate(Err(e)) => {
                panic!("expected ApiRequest, got Immediate Err: {e:?}")
            }
        }
    }

    // --- list_tools tests ---

    #[test]
    fn list_tools_includes_auth_status() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_mcp_auth_status"));
    }

    #[test]
    fn list_tools_includes_api_search() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_api_search"));
    }

    #[test]
    fn list_tools_includes_api_explain() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_api_explain"));
    }

    #[test]
    fn list_tools_includes_api_call() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_api_call"));
    }

    #[test]
    fn list_tools_has_four_tools() {
        let tools = list_tools();
        assert_eq!(tools.len(), 4);
    }

    // --- auth_status tests ---

    #[test]
    fn call_tool_auth_status_returns_not_configured() {
        let config = no_creds();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));
        assert_eq!(call_result.content.len(), 1);

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
    }

    #[test]
    fn call_tool_auth_status_returns_not_validated_with_creds() {
        let config = both_creds();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_validated");
    }

    #[test]
    fn call_tool_auth_status_returns_partial_with_missing_key() {
        let config = partial_creds_missing_secret();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
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
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
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
        let err = assert_immediate_err(call_tool("unknown_tool", None, &config, None));
        assert_eq!(err.code, ErrorCode::METHOD_NOT_FOUND);
        assert!(err.message.contains("unknown_tool"));
    }

    #[test]
    fn call_tool_auth_status_ignores_unexpected_arguments() {
        let config = no_creds();
        let mut args = Map::new();
        args.insert("unexpected".to_string(), Value::String("value".to_string()));
        // Extra arguments are silently ignored, consistent with the API
        // tools which use serde's default lenient deserialization.
        let call_result = assert_immediate_ok(call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &config,
            None,
        ));
        assert_eq!(call_result.is_error, Some(false));
        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
    }

    #[test]
    fn error_code_values() {
        // Verify rmcp's error codes match JSON-RPC spec
        assert_eq!(ErrorCode::METHOD_NOT_FOUND.0, -32601);
        assert_eq!(ErrorCode::INVALID_PARAMS.0, -32602);
        assert_eq!(ErrorCode::INTERNAL_ERROR.0, -32603);
    }

    // ====================================================================
    // OAuth Tool Tests
    // ====================================================================

    fn oauth_no_creds() -> AuthConfig {
        AuthConfig {
            method: onshape_client_core::auth::AuthMethod::OAuth,
            ..AuthConfig::default()
        }
    }

    fn oauth_configured() -> AuthConfig {
        AuthConfig {
            client_id: Some("my-client-id".into()),
            client_secret: Some(SecretString::from("my-client-secret")),
            method: onshape_client_core::auth::AuthMethod::OAuth,
            ..AuthConfig::default()
        }
    }

    fn oauth_partial_missing_secret() -> AuthConfig {
        AuthConfig {
            client_id: Some("my-client-id".into()),
            client_secret: None,
            method: onshape_client_core::auth::AuthMethod::OAuth,
            ..AuthConfig::default()
        }
    }

    fn oauth_partial_missing_id() -> AuthConfig {
        AuthConfig {
            client_id: None,
            client_secret: Some(SecretString::from("my-client-secret")),
            method: onshape_client_core::auth::AuthMethod::OAuth,
            ..AuthConfig::default()
        }
    }

    #[test]
    fn call_tool_auth_status_oauth_not_configured_no_creds() {
        let config = oauth_no_creds();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert_eq!(value["auth_method"], "oauth");
    }

    #[test]
    fn call_tool_auth_status_oauth_configured() {
        let config = oauth_configured();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert_eq!(value["auth_method"], "oauth");
        assert!(
            value["message"]
                .as_str()
                .is_some_and(|m| m.contains("no access token"))
        );
    }

    #[test]
    fn call_tool_auth_status_oauth_partial_missing_secret() {
        let config = oauth_partial_missing_secret();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert!(
            value["message"]
                .as_str()
                .is_some_and(|m| m.contains("client_secret"))
        );
    }

    #[test]
    fn call_tool_auth_status_oauth_partial_missing_id() {
        let config = oauth_partial_missing_id();
        let result = call_tool("onshape_mcp_auth_status", None, &config, None);
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert!(
            value["message"]
                .as_str()
                .is_some_and(|m| m.contains("client_id"))
        );
    }

    // --- api_search tests ---

    #[test]
    fn api_search_returns_results() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String("document".to_string()));

        let result = call_tool("onshape_api_search", Some(&args), &config, Some(&spec));
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let results: Vec<Value> = serde_json::from_str(&text.text).expect("should be JSON array");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn api_search_empty_query_returns_all() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String(String::new()));

        let result = call_tool("onshape_api_search", Some(&args), &config, Some(&spec));
        let call_result = assert_immediate_ok(result);

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let results: Vec<Value> = serde_json::from_str(&text.text).expect("should be JSON array");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn api_search_without_spec_returns_error() {
        let config = no_creds();
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String("test".to_string()));

        let err = assert_immediate_err(call_tool("onshape_api_search", Some(&args), &config, None));
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    // --- api_explain tests ---

    #[test]
    fn api_explain_returns_detail() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocuments".to_string()),
        );

        let result = call_tool("onshape_api_explain", Some(&args), &config, Some(&spec));
        let call_result = assert_immediate_ok(result);

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let detail: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(detail["operation_id"], "getDocuments");
        assert_eq!(detail["method"], "GET");
    }

    #[test]
    fn api_explain_nonexistent_returns_error() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("nonexistent".to_string()),
        );

        let err = assert_immediate_err(call_tool(
            "onshape_api_explain",
            Some(&args),
            &config,
            Some(&spec),
        ));
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    // --- api_call tests ---

    #[test]
    fn api_call_returns_request_effect() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocument".to_string()),
        );
        let mut path_params = Map::new();
        path_params.insert("did".to_string(), Value::String("abc123".to_string()));
        args.insert("path_params".to_string(), Value::Object(path_params));

        let result = call_tool("onshape_api_call", Some(&args), &config, Some(&spec));
        let request = assert_api_request(result);
        assert_eq!(request.path, "/documents/abc123");
    }

    #[test]
    fn api_call_missing_required_param_returns_error() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocument".to_string()),
        );
        // Missing required path param "did"

        let err = assert_immediate_err(call_tool(
            "onshape_api_call",
            Some(&args),
            &config,
            Some(&spec),
        ));
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn api_call_without_spec_returns_error() {
        let config = no_creds();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocument".to_string()),
        );

        let err = assert_immediate_err(call_tool("onshape_api_call", Some(&args), &config, None));
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn api_call_with_body_string_returns_request() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert(
            "body".to_string(),
            Value::String(r#"{"rawQuery": "cabinets", "limit": 5}"#.to_string()),
        );

        let result = call_tool("onshape_api_call", Some(&args), &config, Some(&spec));
        let request = assert_api_request(result);
        assert_eq!(request.path, "/documents/search");

        let body = request.body.expect("request should have a body");
        assert_eq!(body["rawQuery"], "cabinets");
        assert_eq!(body["limit"], 5);
        assert_eq!(
            request.content_type.as_deref(),
            Some("application/json;charset=UTF-8; qs=0.09")
        );
    }

    #[test]
    fn api_call_with_invalid_body_json_returns_error() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert(
            "body".to_string(),
            Value::String("not valid json".to_string()),
        );

        let err = assert_immediate_err(call_tool(
            "onshape_api_call",
            Some(&args),
            &config,
            Some(&spec),
        ));
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("invalid body JSON"));
    }

    #[test]
    fn api_call_with_null_body_json_returns_error() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert("body".to_string(), Value::String("null".to_string()));

        let err = assert_immediate_err(call_tool(
            "onshape_api_call",
            Some(&args),
            &config,
            Some(&spec),
        ));
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("JSON null"));
    }

    #[test]
    fn api_call_with_body_for_get_endpoint_passes_through() {
        let config = no_creds();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocuments".to_string()),
        );
        args.insert(
            "body".to_string(),
            Value::String(r#"{"unexpected": "data"}"#.to_string()),
        );

        let result = call_tool("onshape_api_call", Some(&args), &config, Some(&spec));
        let request = assert_api_request(result);
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/documents");
        assert!(
            request.body.is_some(),
            "body should be silently passed through for GET endpoint without requestBody"
        );
        assert!(
            request.content_type.is_none(),
            "content_type should be None since the endpoint declares no requestBody"
        );
    }

    // --- process_api_response tests ---

    #[test]
    fn process_api_response_success_json() {
        let body = r#"{"id": "abc123", "name": "Test"}"#;
        let result = process_api_response(200, body).expect("should succeed");
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn process_api_response_success_plain_text() {
        let result = process_api_response(200, "plain text response").expect("should succeed");
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn process_api_response_error() {
        let result = process_api_response(404, "Not found").expect("should succeed");
        assert_eq!(result.is_error, Some(true));
    }
}
