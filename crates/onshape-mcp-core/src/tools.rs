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
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;

use rmcp::{
    ErrorData,
    model::{CallToolResult, Content, ErrorCode, Tool, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::ResolvedAuth;
use crate::openapi::{ApiRequest, OpenApiSpec, SearchFilters};
use crate::{AuthStatusResult, ValidationState};

// ============================================================================
// Effect Type
// ============================================================================

/// Side effects that a tool callback can request the I/O layer to apply.
///
/// Returned alongside a [`ToolResult`] from callbacks in
/// [`ToolResult::OnshapeApiRequestThen`]. The I/O layer is responsible
/// for applying these effects after processing the callback's result.
pub enum SideEffect {
    /// Update the runtime credential validation state.
    UpdateValidation(ValidationState),
}

// ============================================================================
// File Write Effect Types
// ============================================================================

/// A file to be written to disk by the I/O layer.
///
/// This is a pure data description of the write — no I/O is performed here.
pub struct FileWrite {
    /// The target file path.
    pub path: PathBuf,
    /// The file contents (raw bytes).
    pub data: Vec<u8>,
}

/// The outcome of a single file write attempt, reported by the I/O layer.
pub enum FileWriteResult {
    /// The file was written successfully.
    Success {
        /// The path that was written.
        path: PathBuf,
    },
    /// The file write failed.
    Error {
        /// The path that was attempted.
        path: PathBuf,
        /// Human-readable error message.
        message: String,
    },
}

/// Callback type for formatting the result of file writes.
///
/// Receives the outcomes of each file write and produces the final
/// [`CallToolResult`] to return to the caller.
pub type WriteFilesFormatter = Box<dyn FnOnce(&[FileWriteResult]) -> CallToolResult + Send>;

/// How the OAuth login flow should operate.
///
/// Returned as part of [`ToolResult::OAuthLoginFlow`] for the I/O layer
/// to execute.
#[derive(Clone, Debug)]
pub enum LoginMode {
    /// Use the OAuth proxy for token exchange.
    ///
    /// The proxy holds the client secret. The CLI fetches the client ID
    /// from the proxy's `/config` endpoint.
    Proxy {
        /// Base URL of the OAuth proxy.
        proxy_url: String,
    },
    /// Exchange tokens directly with Onshape using client credentials.
    Direct {
        /// OAuth 2.0 client ID.
        client_id: String,
        /// OAuth 2.0 client secret.
        client_secret: String,
    },
}

/// Result of dispatching a tool call.
///
/// Tools that need no I/O return `Immediate`. Tools that need an HTTP request
/// to the Onshape API return `OnshapeApiRequest`, which the I/O layer must
/// execute and then pass back through [`process_api_response`].
/// `OnshapeApiRequestThen` extends this with a callback that processes the
/// response and can return further results plus side effects.
/// `OAuthLoginFlow` signals the I/O layer to start an OAuth login flow.
pub enum ToolResult {
    /// Tool completed immediately with no I/O needed.
    Immediate(Result<CallToolResult, ErrorData>),
    /// Tool needs an HTTP request to the Onshape API.
    OnshapeApiRequest {
        /// The HTTP request to execute.
        request: ApiRequest,
    },
    /// Tool needs an HTTP request; the response is processed by a callback
    /// which can return further results and request side effects.
    OnshapeApiRequestThen {
        /// The HTTP request to execute.
        request: ApiRequest,
        /// Callback receives (`http_status`, `response_body`) and returns
        /// the next [`ToolResult`] plus any side effects for the I/O layer.
        #[allow(clippy::type_complexity)]
        then: Box<dyn FnOnce(u16, &str) -> (Self, Vec<SideEffect>) + Send>,
    },
    /// Tool requests an OAuth login flow to be started.
    ///
    /// The I/O layer handles all the I/O: starting a callback server,
    /// building the authorization URL, waiting for the callback, exchanging
    /// the code for tokens, and writing the token file.
    OAuthLoginFlow {
        /// The login mode (proxy or direct).
        mode: LoginMode,
    },
    /// Tool needs files written to disk.
    ///
    /// The I/O layer writes each [`FileWrite`] and reports outcomes via
    /// [`FileWriteResult`]. The `format` callback then produces the final
    /// [`CallToolResult`] based on what succeeded or failed.
    WriteFiles {
        /// Files to write.
        files: Vec<FileWrite>,
        /// Callback that formats the final tool result from write outcomes.
        format: WriteFilesFormatter,
    },
}

/// Create a [`ToolResult`] for an expected user-input error.
///
/// Returns a successful `CallToolResult` with `is_error: Some(true)`, keeping
/// the MCP transport clean. Use this for validation failures that the caller
/// (typically an LLM) can act on — as opposed to protocol-level
/// `Err(ErrorData)` which signals handler/infrastructure breakage.
fn tool_input_error(message: impl Into<String>) -> ToolResult {
    ToolResult::Immediate(Ok(CallToolResult {
        content: vec![Content::text(message.into())],
        is_error: Some(true),
        structured_content: None,
        meta: None,
    }))
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
        tool_auth_login_def(),
        tool_api_search_def(),
        tool_api_explain_def(),
        tool_api_call_def(),
        tool_list_resources_def(),
        tool_read_resource_def(),
        tool_screenshot_def(),
    ]
}

/// Dispatches a tool call by name.
///
/// The `resolved_auth` parameter provides the current authentication state,
/// as determined by the I/O layer from config + token file probe. The
/// `validation` parameter provides runtime credential validation state.
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
    resolved_auth: &ResolvedAuth,
    validation: &ValidationState,
    spec: Option<&OpenApiSpec>,
) -> ToolResult {
    match name {
        "onshape_mcp_auth_status" => call_auth_status(arguments, resolved_auth, validation, spec),
        "onshape_mcp_auth_login" => call_auth_login(arguments),
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
        "onshape_list_resources" => ToolResult::Immediate(Ok(call_list_resources())),
        "onshape_read_resource" => ToolResult::Immediate(call_read_resource(arguments)),
        "onshape_screenshot" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolResult::Immediate(Err(e)),
            };
            call_screenshot(arguments, spec)
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

/// Input schema for `onshape_mcp_auth_status`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct AuthStatusInput {
    /// When true, actively validates credentials against the Onshape API
    /// by calling GET /users/sessioninfo. When false or omitted, returns
    /// the cached validation state without making any API calls.
    #[serde(default)]
    pub validate: Option<bool>,
}

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

/// Input schema for `onshape_mcp_auth_login`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct AuthLoginInput {
    /// Login mode: `"proxy"` (default) uses the OAuth proxy for token exchange;
    /// `"direct"` exchanges tokens directly with Onshape using client credentials.
    #[serde(default)]
    pub mode: Option<String>,
    /// OAuth proxy URL override. Only used in proxy mode.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// OAuth 2.0 client ID. Required for direct mode.
    #[serde(default)]
    pub client_id: Option<String>,
    /// OAuth 2.0 client secret. Required for direct mode.
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Input schema for `onshape_read_resource`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadResourceInput {
    /// The URI of the resource to read (e.g., `"insights:shaded-views"`).
    /// Use `onshape_list_resources` to discover available URIs.
    pub uri: String,
}

/// Input schema for `onshape_screenshot`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    /// Document ID.
    pub did: String,
    /// Context type: `"w"` (workspace), `"v"` (version), or `"m"` (microversion).
    pub wvm: String,
    /// Workspace, version, or microversion ID.
    pub wvmid: String,
    /// Part Studio element ID.
    pub eid: String,

    /// View specification (named preset or custom angles).
    pub view: ViewSpec,

    /// Full file path for the output PNG (e.g., `"/tmp/screenshot.png"`).
    pub output_path: String,

    /// Image height in pixels. Defaults to 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_height: Option<u32>,
    /// Image width in pixels. Defaults to 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_width: Option<u32>,
    /// Edge visibility: `"show"` (default) or `"hide"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<String>,
    /// Enable anti-aliasing for smoother edges. Defaults to false.
    /// Can cause failures on very large images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_anti_aliasing: Option<bool>,
    /// Show all parts regardless of user visibility settings. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_all_parts: Option<bool>,
    /// Include surfaces (only effective when `show_all_parts` is true). Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_surfaces: Option<bool>,
    /// Include wire bodies. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_wires: Option<bool>,
}

/// A view specification for a screenshot. Either a named preset or custom
/// azimuth/elevation angles.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ViewSpec {
    /// A named view preset.
    #[serde(rename = "preset")]
    Preset {
        /// The preset name.
        name: ViewPreset,
    },
    /// Custom view angles (orbit around the model).
    #[serde(rename = "angles")]
    Angles {
        /// Horizontal orbit angle in degrees. 0 = front, 90 = right,
        /// 180 = back, 270 = left.
        azimuth: f64,
        /// Vertical tilt in degrees above the horizontal plane.
        /// 0 = horizontal, 90 = top-down, -90 = bottom-up.
        elevation: f64,
    },
}

/// Named view presets for Part Studio screenshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewPreset {
    /// Front view (looking along -Y).
    Front,
    /// Back view (looking along +Y).
    Back,
    /// Top view (looking along -Z).
    Top,
    /// Bottom view (looking along +Z).
    Bottom,
    /// Left view (looking along +X).
    Left,
    /// Right view (looking along -X).
    Right,
    /// Isometric view (~azimuth 45°, elevation 35.26°).
    Isometric,
}

// ============================================================================
// Tool Definitions
// ============================================================================

#[allow(clippy::expect_used)] // Schema generation should never fail
fn tool_auth_status_def() -> Tool {
    let schema = schemars::schema_for!(AuthStatusInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("AuthStatusInput schema serialization should never fail");
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
    .annotate(ToolAnnotations::new().read_only(false).destructive(false))
}

#[allow(clippy::expect_used)]
fn tool_auth_login_def() -> Tool {
    let schema = schemars::schema_for!(AuthLoginInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("AuthLoginInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_mcp_auth_login",
        "Start an OAuth authorization flow. Returns a URL to open in your browser. \
         After authorizing, the server automatically detects the new tokens.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(false).destructive(false))
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

#[allow(clippy::expect_used)]
fn tool_list_resources_def() -> Tool {
    let schema = schemars::schema_for!(EmptyInput);
    let input_schema: Value =
        serde_json::to_value(schema).expect("EmptyInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_list_resources",
        "List available resource documents with practical Onshape API guidance. \
         Returns URIs, titles, and descriptions. Use onshape_read_resource to \
         read a specific resource by URI.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

#[allow(clippy::expect_used)]
fn tool_read_resource_def() -> Tool {
    let schema = schemars::schema_for!(ReadResourceInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("ReadResourceInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_read_resource",
        "Read a specific resource document by URI. Returns the full markdown content \
         with practical guidance for Onshape API usage. Use onshape_list_resources \
         to discover available URIs.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

// ============================================================================
// Tool Implementations
// ============================================================================

fn call_auth_status(
    arguments: Option<&Map<String, Value>>,
    resolved_auth: &ResolvedAuth,
    validation: &ValidationState,
    spec: Option<&OpenApiSpec>,
) -> ToolResult {
    let input: AuthStatusInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };

    if input.validate != Some(true) {
        // Return cached validation state — no API call needed.
        let now = chrono::Utc::now();
        let result = AuthStatusResult::new(resolved_auth, Some(validation), now);
        let content = match Content::json(&result) {
            Ok(c) => c,
            Err(e) => return ToolResult::Immediate(Err(e)),
        };
        return ToolResult::Immediate(Ok(CallToolResult {
            content: vec![content],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        }));
    }

    // validate=true: actively check credentials via GET /users/sessioninfo.
    let spec = match require_spec(spec) {
        Ok(s) => s,
        Err(e) => return ToolResult::Immediate(Err(e)),
    };

    let empty_map = HashMap::new();
    let request = match spec.build_request("sessionInfo", &empty_map, &empty_map, None) {
        Ok(req) => req,
        Err(e) => {
            return ToolResult::Immediate(Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("failed to build sessionInfo request: {e}"),
                None,
            )));
        }
    };

    // Capture what we need for the callback closure.
    let resolved_auth = resolved_auth.clone();

    ToolResult::OnshapeApiRequestThen {
        request,
        then: Box::new(move |status, _body| {
            let now = chrono::Utc::now();

            if (200..300).contains(&status) {
                let valid_state = ValidationState {
                    status: crate::ValidationStatus::Valid,
                    last_check: Some(now),
                    message: Some("Credentials validated successfully".into()),
                };
                let result = AuthStatusResult::new(&resolved_auth, Some(&valid_state), now);
                let tool_result = match Content::json(&result) {
                    Ok(c) => ToolResult::Immediate(Ok(CallToolResult {
                        content: vec![c],
                        is_error: Some(false),
                        structured_content: None,
                        meta: None,
                    })),
                    Err(e) => ToolResult::Immediate(Err(e)),
                };
                (tool_result, vec![SideEffect::UpdateValidation(valid_state)])
            } else if status == 401 {
                let invalid_state = ValidationState {
                    status: crate::ValidationStatus::Invalid,
                    last_check: Some(now),
                    message: Some("API returned 401 Unauthorized — credentials are invalid".into()),
                };
                let result = AuthStatusResult::new(&resolved_auth, Some(&invalid_state), now);
                let tool_result = match Content::json(&result) {
                    Ok(c) => ToolResult::Immediate(Ok(CallToolResult {
                        content: vec![c],
                        is_error: Some(false),
                        structured_content: None,
                        meta: None,
                    })),
                    Err(e) => ToolResult::Immediate(Err(e)),
                };
                (
                    tool_result,
                    vec![SideEffect::UpdateValidation(invalid_state)],
                )
            } else {
                // Unexpected status — don't update validation state.
                let result = AuthStatusResult::new(&resolved_auth, None, now);
                let mut auth_result = match Content::json(&result) {
                    Ok(c) => CallToolResult {
                        content: vec![c],
                        is_error: Some(false),
                        structured_content: None,
                        meta: None,
                    },
                    Err(e) => {
                        return (ToolResult::Immediate(Err(e)), vec![]);
                    }
                };
                // Add a note about the unexpected status.
                auth_result.content.push(Content::text(format!(
                    "Warning: credential validation returned unexpected HTTP {status}"
                )));
                (ToolResult::Immediate(Ok(auth_result)), vec![])
            }
        }),
    }
}

/// Default OAuth proxy URL used when no `proxy_url` is specified.
pub const DEFAULT_PROXY_URL: &str = "https://onshape-oauth-proxy.fstab.workers.dev";

fn call_auth_login(arguments: Option<&Map<String, Value>>) -> ToolResult {
    let input: AuthLoginInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };

    let mode_str = input.mode.as_deref().unwrap_or("proxy");

    let mode = match mode_str {
        "proxy" => {
            let proxy_url = input
                .proxy_url
                .unwrap_or_else(|| DEFAULT_PROXY_URL.to_string());
            LoginMode::Proxy { proxy_url }
        }
        "direct" => {
            let Some(client_id) = input.client_id else {
                return tool_input_error("client_id is required for direct mode");
            };
            let Some(client_secret) = input.client_secret else {
                return tool_input_error("client_secret is required for direct mode");
            };
            LoginMode::Direct {
                client_id,
                client_secret,
            }
        }
        other => {
            return tool_input_error(format!(
                "invalid mode \"{other}\": expected \"proxy\" (default) or \"direct\""
            ));
        }
    };

    ToolResult::OAuthLoginFlow { mode }
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
    let detail = match spec.explain(&input.endpoint) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult {
                content: vec![Content::text(format!("{e}"))],
                is_error: Some(true),
                structured_content: None,
                meta: None,
            });
        }
    };

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
        Err(e) => return tool_input_error(e.message),
    };

    let body: Option<Value> = match input.body.as_deref().map(serde_json::from_str).transpose() {
        Ok(v) => v,
        Err(e) => {
            return tool_input_error(format!("invalid body JSON: {e}"));
        }
    };

    if body == Some(Value::Null) {
        return tool_input_error(
            "body parsed as JSON null; omit the body field instead of passing \"null\"",
        );
    }

    let request = match spec.build_request(
        &input.endpoint,
        &input.path_params,
        &input.query_params,
        body,
    ) {
        Ok(req) => req,
        Err(e) => {
            return tool_input_error(format!("{e}"));
        }
    };

    ToolResult::OnshapeApiRequest { request }
}

fn call_list_resources() -> CallToolResult {
    use std::fmt::Write;

    let resources = onshape_mcp_resources::RESOURCES;
    let mut output = format!("Available resources ({}):\n", resources.len());

    for entry in resources {
        // Writing to String is infallible (std::fmt::Write for String always returns Ok).
        let _ = write!(
            output,
            "\n{} — {}\n  {}\n",
            entry.uri, entry.title, entry.description
        );
    }

    CallToolResult {
        content: vec![Content::text(output)],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    }
}

fn call_read_resource(arguments: Option<&Map<String, Value>>) -> Result<CallToolResult, ErrorData> {
    let input: ReadResourceInput = parse_arguments(arguments)?;

    let entry = onshape_mcp_resources::RESOURCES
        .iter()
        .find(|e| e.uri == input.uri);

    if let Some(entry) = entry {
        Ok(CallToolResult {
            content: vec![Content::text(entry.content)],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        })
    } else {
        let available: Vec<&str> = onshape_mcp_resources::RESOURCES
            .iter()
            .map(|e| e.uri)
            .collect();
        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Resource not found: {}. Available URIs: {}",
                input.uri,
                available.join(", ")
            ))],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        })
    }
}

// ============================================================================
// Screenshot Tool
// ============================================================================

/// Compute the `viewMatrix` parameter string for a [`ViewSpec`].
///
/// Returns either a named preset string (e.g. `"front"`) or a comma-separated
/// 12-number rotation matrix for the Onshape shaded views API.
///
/// The Onshape view matrix is a 3×4 row-major matrix that transforms model
/// coordinates to view coordinates:
/// - View x = right, y = up, z = toward viewer
/// - Model x = right, y = forward (into screen in front view), z = up
fn view_matrix_string(spec: &ViewSpec) -> String {
    match spec {
        ViewSpec::Preset { name } => match name {
            ViewPreset::Front => "front".to_string(),
            ViewPreset::Back => "back".to_string(),
            ViewPreset::Top => "top".to_string(),
            ViewPreset::Bottom => "bottom".to_string(),
            ViewPreset::Left => "left".to_string(),
            ViewPreset::Right => "right".to_string(),
            ViewPreset::Isometric => {
                // Approximate isometric: azimuth=45°, elevation=35.264° (arctan(1/√2))
                view_matrix_from_angles(45.0, 35.264_389_682_754_654)
            }
        },
        ViewSpec::Angles { azimuth, elevation } => view_matrix_from_angles(*azimuth, *elevation),
    }
}

/// Compute a 12-number view matrix string from azimuth and elevation angles.
///
/// - `azimuth`: horizontal orbit in degrees (0 = front, 90 = right, 180 = back, 270 = left)
/// - `elevation`: vertical tilt in degrees above horizontal (0 = level, 90 = top-down)
///
/// The matrix is returned as 12 comma-separated floats (3 rows × 4 columns, row-major).
///
/// ## Derivation
///
/// We build a camera rotation that orbits around the model's vertical axis (Z-up)
/// then tilts up/down. The Onshape view matrix M transforms model coords to view
/// coords where (x=right, y=up, z=toward-viewer).
///
/// Starting from the front view (looking along -Y):
/// 1. Rotate around Z by azimuth (camera orbits horizontally)
/// 2. Tilt by elevation (camera looks up/down)
///
/// The resulting view axes (expressed in model space) are:
/// - `view_x` (right): `(-sin(a), cos(a), 0)` — perpendicular to look direction in XY plane
/// - `view_y` (up):    `(-cos(a)*sin(e), -sin(a)*sin(e), cos(e))` — tilted up
/// - `view_z` (toward viewer): `(cos(a)*cos(e), sin(a)*cos(e), sin(e))` — look direction negated
///
/// The view matrix rows are `[view_x | 0]`, `[view_y | 0]`, `[view_z | 0]`
/// (no translation — the API auto-centers on the model).
fn view_matrix_from_angles(azimuth_deg: f64, elevation_deg: f64) -> String {
    let a = azimuth_deg.to_radians();
    let e = elevation_deg.to_radians();

    let (sin_a, cos_a) = a.sin_cos();
    let (sin_e, cos_e) = e.sin_cos();

    // Row 1: view X axis (right) in model coords
    let r00 = -sin_a;
    let r01 = cos_a;
    let r02 = 0.0;

    // Row 2: view Y axis (up) in model coords
    let r10 = -cos_a * sin_e;
    let r11 = -sin_a * sin_e;
    let r12 = cos_e;

    // Row 3: view Z axis (toward viewer) in model coords
    let r20 = cos_a * cos_e;
    let r21 = sin_a * cos_e;
    let r22 = sin_e;

    format!("{r00},{r01},{r02},0,{r10},{r11},{r12},0,{r20},{r21},{r22},0")
}

/// Human-readable label for a view spec, used in result messages.
fn view_label(spec: &ViewSpec) -> String {
    match spec {
        ViewSpec::Preset { name } => match name {
            ViewPreset::Front => "front".to_string(),
            ViewPreset::Back => "back".to_string(),
            ViewPreset::Top => "top".to_string(),
            ViewPreset::Bottom => "bottom".to_string(),
            ViewPreset::Left => "left".to_string(),
            ViewPreset::Right => "right".to_string(),
            ViewPreset::Isometric => "isometric".to_string(),
        },
        ViewSpec::Angles { azimuth, elevation } => {
            format!("azimuth={azimuth}\u{00b0}, elevation={elevation}\u{00b0}")
        }
    }
}

#[allow(clippy::expect_used)]
fn tool_screenshot_def() -> Tool {
    let schema = schemars::schema_for!(ScreenshotInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("ScreenshotInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_screenshot",
        "Take a screenshot of a Part Studio. Renders a single view server-side \
         and saves the PNG to disk. Returns the file path. Always uses auto-fit \
         (pixelSize=0) so parts fill the image. Accepts named view presets \
         (front, back, top, bottom, left, right, isometric) or custom \
         azimuth/elevation angles. Call multiple times for multiple views.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(false).destructive(false))
}

fn call_screenshot(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolResult {
    const MAX_SCREENSHOT_DIM: u32 = 4096;

    let input: ScreenshotInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };

    // --- Validate inputs ---

    if let Some(ref edges) = input.edges
        && edges != "show"
        && edges != "hide"
    {
        return tool_input_error(format!(
            "invalid edges value \"{edges}\": expected \"show\" or \"hide\""
        ));
    }

    let valid_wvm = ["w", "v", "m"];
    if !valid_wvm.contains(&input.wvm.as_str()) {
        return tool_input_error(format!(
            "invalid wvm value \"{}\": expected \"w\", \"v\", or \"m\"",
            input.wvm
        ));
    }

    // --- Build the API request ---

    let view_matrix = view_matrix_string(&input.view);

    let path_params: HashMap<String, String> = [
        ("did".to_string(), input.did.clone()),
        ("wvm".to_string(), input.wvm.clone()),
        ("wvmid".to_string(), input.wvmid.clone()),
        ("eid".to_string(), input.eid.clone()),
    ]
    .into_iter()
    .collect();

    let mut query_params: HashMap<String, String> = HashMap::new();
    query_params.insert("viewMatrix".to_string(), view_matrix.clone());
    query_params.insert("pixelSize".to_string(), "0".to_string());

    if let Some(h) = input.output_height {
        if h == 0 || h > MAX_SCREENSHOT_DIM {
            return tool_input_error(format!(
                "invalid output_height {h}: expected 1..={MAX_SCREENSHOT_DIM}"
            ));
        }
        query_params.insert("outputHeight".to_string(), h.to_string());
    }
    if let Some(w) = input.output_width {
        if w == 0 || w > MAX_SCREENSHOT_DIM {
            return tool_input_error(format!(
                "invalid output_width {w}: expected 1..={MAX_SCREENSHOT_DIM}"
            ));
        }
        query_params.insert("outputWidth".to_string(), w.to_string());
    }
    if let Some(ref edges) = input.edges {
        query_params.insert("edges".to_string(), edges.clone());
    }
    if let Some(aa) = input.use_anti_aliasing {
        query_params.insert("useAntiAliasing".to_string(), aa.to_string());
    }
    if let Some(sap) = input.show_all_parts {
        query_params.insert("showAllParts".to_string(), sap.to_string());
    }
    if let Some(is) = input.include_surfaces {
        query_params.insert("includeSurfaces".to_string(), is.to_string());
    }
    if let Some(iw) = input.include_wires {
        query_params.insert("includeWires".to_string(), iw.to_string());
    }

    // --- Prepare data for the callback closure ---

    let label = view_label(&input.view);
    let output_path = PathBuf::from(&input.output_path);
    if input.output_path.trim().is_empty() || output_path.file_name().is_none() {
        return tool_input_error("output_path must include a file name");
    }
    if output_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return tool_input_error("output_path must not contain '..' segments");
    }

    let request = match spec.build_request(
        "getPartStudioShadedViews",
        &path_params,
        &query_params,
        None,
    ) {
        Ok(req) => req,
        Err(e) => {
            return tool_input_error(format!("failed to build shaded views request: {e}"));
        }
    };

    // --- Return the two-phase effect: API call, then file writes ---

    ToolResult::OnshapeApiRequestThen {
        request,
        then: Box::new(move |status, body| {
            process_screenshot_response(status, body, output_path, label, view_matrix)
        }),
    }
}

/// Process the shaded views API response: decode the base64 image and produce
/// a [`ToolResult::WriteFiles`] effect.
///
/// Extracted from [`call_screenshot`] to keep function lengths manageable.
fn process_screenshot_response(
    status: u16,
    body: &str,
    output_path: PathBuf,
    label: String,
    view_matrix: String,
) -> (ToolResult, Vec<SideEffect>) {
    if !(200..300).contains(&status) {
        return (
            ToolResult::Immediate(Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Shaded views API error (HTTP {status}): {body}"
                ))],
                is_error: Some(true),
                structured_content: None,
                meta: None,
            })),
            vec![],
        );
    }

    // Parse the response JSON.
    let response: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                tool_input_error(format!("failed to parse shaded views response: {e}")),
                vec![],
            );
        }
    };

    // Extract the first image from the images array.
    // Response shape: { "images": ["base64string"] }
    let Some(images) = response.get("images").and_then(Value::as_array) else {
        return (
            tool_input_error("shaded views response missing \"images\" array"),
            vec![],
        );
    };

    let Some(first_image) = images.first() else {
        return (
            tool_input_error("shaded views response returned empty \"images\" array"),
            vec![],
        );
    };

    let Some(b64_str) = first_image.as_str() else {
        return (
            tool_input_error("first image in response is not a string"),
            vec![],
        );
    };

    // Decode base64 image into bytes.
    let engine = base64::engine::general_purpose::STANDARD;
    let data = match engine.decode(b64_str) {
        Ok(d) => d,
        Err(e) => {
            return (
                tool_input_error(format!("failed to decode base64 image: {e}")),
                vec![],
            );
        }
    };

    let file_write = FileWrite {
        path: output_path,
        data,
    };

    (
        ToolResult::WriteFiles {
            files: vec![file_write],
            format: Box::new(move |results: &[FileWriteResult]| {
                let Some(result) = results.first() else {
                    return CallToolResult {
                        content: vec![Content::text("internal error: no file write results")],
                        is_error: Some(true),
                        structured_content: None,
                        meta: None,
                    };
                };
                format_screenshot_result(result, &label, &view_matrix)
            }),
        },
        vec![],
    )
}

/// Format the final [`CallToolResult`] for the screenshot tool.
///
/// Produces both structured JSON and a human-readable summary.
fn format_screenshot_result(
    result: &FileWriteResult,
    label: &str,
    view_matrix: &str,
) -> CallToolResult {
    match result {
        FileWriteResult::Success { path } => {
            let structured = serde_json::json!({
                "path": path.display().to_string(),
                "view": label,
                "view_matrix": view_matrix,
                "status": "ok"
            });
            let summary = format!(
                "Saved screenshot: {} ({label}, viewMatrix={view_matrix})",
                path.display()
            );
            CallToolResult {
                content: vec![
                    Content::json(&structured)
                        .unwrap_or_else(|_| Content::text(structured.to_string())),
                    Content::text(summary),
                ],
                is_error: Some(false),
                structured_content: None,
                meta: None,
            }
        }
        FileWriteResult::Error { path, message } => {
            let structured = serde_json::json!({
                "path": path.display().to_string(),
                "view": label,
                "view_matrix": view_matrix,
                "status": "error",
                "error": message
            });
            let summary = format!(
                "FAILED to save screenshot: {} ({label}, viewMatrix={view_matrix}) -- {message}",
                path.display()
            );
            CallToolResult {
                content: vec![
                    Content::json(&structured)
                        .unwrap_or_else(|_| Content::text(structured.to_string())),
                    Content::text(summary),
                ],
                is_error: Some(true),
                structured_content: None,
                meta: None,
            }
        }
    }
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
    use super::*;
    use crate::{ValidationState, openapi::HttpMethod};

    fn default_validation() -> ValidationState {
        ValidationState::default()
    }

    fn not_configured() -> ResolvedAuth {
        ResolvedAuth::NotConfigured {
            configured_method: onshape_client_core::auth::AuthMethod::Auto,
            detail: "No credentials configured".into(),
        }
    }

    fn basic_ready() -> ResolvedAuth {
        ResolvedAuth::Basic
    }

    fn not_configured_partial_secret() -> ResolvedAuth {
        ResolvedAuth::NotConfigured {
            configured_method: onshape_client_core::auth::AuthMethod::Basic,
            detail: "Incomplete credentials: secret_key is not configured".into(),
        }
    }

    fn not_configured_partial_access() -> ResolvedAuth {
        ResolvedAuth::NotConfigured {
            configured_method: onshape_client_core::auth::AuthMethod::Basic,
            detail: "Incomplete credentials: access_key is not configured".into(),
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
            ToolResult::OnshapeApiRequestThen { .. } => {
                panic!("expected Immediate, got ApiRequestThen")
            }
            ToolResult::OAuthLoginFlow { .. } => {
                panic!("expected Immediate, got OAuthLoginFlow")
            }
            ToolResult::WriteFiles { .. } => {
                panic!("expected Immediate, got WriteFiles")
            }
        }
    }

    fn assert_immediate_err(result: ToolResult) -> ErrorData {
        match result {
            ToolResult::Immediate(Err(e)) => e,
            ToolResult::Immediate(Ok(_)) => panic!("expected Err, got Ok"),
            ToolResult::OnshapeApiRequest { .. } => panic!("expected Immediate, got ApiRequest"),
            ToolResult::OnshapeApiRequestThen { .. } => {
                panic!("expected Immediate, got ApiRequestThen")
            }
            ToolResult::OAuthLoginFlow { .. } => {
                panic!("expected Immediate Err, got OAuthLoginFlow")
            }
            ToolResult::WriteFiles { .. } => {
                panic!("expected Immediate Err, got WriteFiles")
            }
        }
    }

    /// Asserts that `result` is a tool-level error (`is_error: Some(true)`)
    /// and returns the concatenated text content for further assertions.
    fn assert_tool_error(result: ToolResult) -> String {
        match result {
            ToolResult::Immediate(Ok(r)) => {
                assert_eq!(
                    r.is_error,
                    Some(true),
                    "expected is_error=true, got {:?}",
                    r.is_error
                );
                r.content
                    .iter()
                    .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                    .collect::<String>()
            }
            ToolResult::Immediate(Err(e)) => {
                panic!("expected tool error (is_error=true), got protocol error: {e:?}")
            }
            other => panic!(
                "expected Immediate tool error, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    fn assert_api_request(result: ToolResult) -> ApiRequest {
        match result {
            ToolResult::OnshapeApiRequest { request } => request,
            ToolResult::Immediate(Ok(_)) => panic!("expected ApiRequest, got Immediate Ok"),
            ToolResult::Immediate(Err(e)) => {
                panic!("expected ApiRequest, got Immediate Err: {e:?}")
            }
            ToolResult::OnshapeApiRequestThen { .. } => {
                panic!("expected ApiRequest, got ApiRequestThen")
            }
            ToolResult::OAuthLoginFlow { .. } => {
                panic!("expected ApiRequest, got OAuthLoginFlow")
            }
            ToolResult::WriteFiles { .. } => {
                panic!("expected ApiRequest, got WriteFiles")
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
    fn list_tools_includes_list_resources() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_list_resources"));
    }

    #[test]
    fn list_tools_includes_read_resource() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_read_resource"));
    }

    #[test]
    fn list_tools_includes_auth_login() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_mcp_auth_login"));
    }

    #[test]
    fn list_tools_has_eight_tools() {
        let tools = list_tools();
        assert_eq!(tools.len(), 8);
    }

    // --- auth_status tests ---

    #[test]
    fn call_tool_auth_status_returns_not_configured() {
        let auth = not_configured();
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
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
        let auth = basic_ready();
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_validated");
    }

    #[test]
    fn call_tool_auth_status_returns_partial_with_missing_key() {
        let auth = not_configured_partial_secret();
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
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
        let auth = not_configured_partial_access();
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
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
        let auth = not_configured();
        let err = assert_immediate_err(call_tool(
            "unknown_tool",
            None,
            &auth,
            &default_validation(),
            None,
        ));
        assert_eq!(err.code, ErrorCode::METHOD_NOT_FOUND);
        assert!(err.message.contains("unknown_tool"));
    }

    #[test]
    fn call_tool_auth_status_ignores_unexpected_arguments() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("unexpected".to_string(), Value::String("value".to_string()));
        // Extra arguments are silently ignored, consistent with the API
        // tools which use serde's default lenient deserialization.
        let call_result = assert_immediate_ok(call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
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

    #[test]
    fn call_tool_auth_status_oauth_not_configured_no_creds() {
        let auth = ResolvedAuth::NotConfigured {
            configured_method: onshape_client_core::auth::AuthMethod::OAuth,
            detail: "No credentials configured".into(),
        };
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_configured");
        assert_eq!(value["auth_method"], "oauth");
    }

    #[test]
    fn call_tool_auth_status_oauth_pending() {
        let auth = ResolvedAuth::OAuthPending;
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
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
    fn call_tool_auth_status_oauth_ready() {
        let auth = ResolvedAuth::OAuthReady { expires_at: None };
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "not_validated");
        assert_eq!(value["auth_method"], "oauth");
    }

    #[test]
    fn call_tool_auth_status_oauth_partial_missing_secret() {
        let auth = ResolvedAuth::NotConfigured {
            configured_method: onshape_client_core::auth::AuthMethod::OAuth,
            detail: "Incomplete credentials: client_secret is not configured".into(),
        };
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
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
        let auth = ResolvedAuth::NotConfigured {
            configured_method: onshape_client_core::auth::AuthMethod::OAuth,
            detail: "Incomplete credentials: client_id is not configured".into(),
        };
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
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
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String("document".to_string()));

        let result = call_tool(
            "onshape_api_search",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let results: Vec<Value> = serde_json::from_str(&text.text).expect("should be JSON array");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn api_search_empty_query_returns_all() {
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String(String::new()));

        let result = call_tool(
            "onshape_api_search",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let call_result = assert_immediate_ok(result);

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let results: Vec<Value> = serde_json::from_str(&text.text).expect("should be JSON array");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn api_search_without_spec_returns_error() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String("test".to_string()));

        let err = assert_immediate_err(call_tool(
            "onshape_api_search",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        ));
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    // --- api_explain tests ---

    #[test]
    fn api_explain_returns_detail() {
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocuments".to_string()),
        );

        let result = call_tool(
            "onshape_api_explain",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let call_result = assert_immediate_ok(result);

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let detail: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(detail["operation_id"], "getDocuments");
        assert_eq!(detail["method"], "GET");
    }

    #[test]
    fn api_explain_nonexistent_returns_error() {
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("nonexistent".to_string()),
        );

        let result = assert_immediate_ok(call_tool(
            "onshape_api_explain",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert_eq!(result.is_error, Some(true));
    }

    // --- api_call tests ---

    #[test]
    fn api_call_returns_request_effect() {
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocument".to_string()),
        );
        let mut path_params = Map::new();
        path_params.insert("did".to_string(), Value::String("abc123".to_string()));
        args.insert("path_params".to_string(), Value::Object(path_params));

        let result = call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let request = assert_api_request(result);
        assert_eq!(request.path, "/documents/abc123");
    }

    #[test]
    fn api_call_missing_required_param_returns_error() {
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocument".to_string()),
        );
        // Missing required path param "did"

        let msg = assert_tool_error(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(!msg.is_empty());
    }

    #[test]
    fn api_call_without_spec_returns_error() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocument".to_string()),
        );

        let err = assert_immediate_err(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        ));
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn api_call_with_body_string_returns_request() {
        let auth = not_configured();
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

        let result = call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
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
        let auth = not_configured();
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

        let msg = assert_tool_error(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains("invalid body JSON"));
    }

    #[test]
    fn api_call_with_null_body_json_returns_error() {
        let auth = not_configured();
        let spec = test_spec();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert("body".to_string(), Value::String("null".to_string()));

        let msg = assert_tool_error(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains("JSON null"));
    }

    #[test]
    fn api_call_with_body_for_get_endpoint_passes_through() {
        let auth = not_configured();
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

        let result = call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
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

    // ====================================================================
    // Auth Status Validate Tests (Feature 3)
    // ====================================================================

    fn test_spec_with_session_info() -> OpenApiSpec {
        OpenApiSpec::from_json(
            r#"{
                "openapi": "3.0.1",
                "info": { "title": "Test API", "version": "1.0" },
                "servers": [{ "url": "https://cad.onshape.com/api/v1" }],
                "paths": {
                    "/users/sessioninfo": {
                        "get": {
                            "operationId": "sessionInfo",
                            "summary": "Get current user session info",
                            "tags": ["User"],
                            "parameters": [],
                            "responses": { "200": {} }
                        }
                    }
                },
                "components": { "schemas": {} }
            }"#,
        )
        .expect("test spec should parse")
    }

    #[allow(clippy::type_complexity)]
    fn assert_api_request_then(
        result: ToolResult,
    ) -> (
        ApiRequest,
        Box<dyn FnOnce(u16, &str) -> (ToolResult, Vec<SideEffect>) + Send>,
    ) {
        match result {
            ToolResult::OnshapeApiRequestThen { request, then } => (request, then),
            ToolResult::Immediate(Ok(_)) => {
                panic!("expected ApiRequestThen, got Immediate Ok")
            }
            ToolResult::Immediate(Err(e)) => {
                panic!("expected ApiRequestThen, got Immediate Err: {e:?}")
            }
            ToolResult::OnshapeApiRequest { .. } => {
                panic!("expected ApiRequestThen, got ApiRequest")
            }
            ToolResult::OAuthLoginFlow { .. } => {
                panic!("expected ApiRequestThen, got OAuthLoginFlow")
            }
            ToolResult::WriteFiles { .. } => {
                panic!("expected ApiRequestThen, got WriteFiles")
            }
        }
    }

    fn assert_oauth_login_flow(result: ToolResult) -> LoginMode {
        match result {
            ToolResult::OAuthLoginFlow { mode } => mode,
            ToolResult::Immediate(Ok(_)) => {
                panic!("expected OAuthLoginFlow, got Immediate Ok")
            }
            ToolResult::Immediate(Err(e)) => {
                panic!("expected OAuthLoginFlow, got Immediate Err: {e:?}")
            }
            ToolResult::OnshapeApiRequest { .. } => {
                panic!("expected OAuthLoginFlow, got ApiRequest")
            }
            ToolResult::OnshapeApiRequestThen { .. } => {
                panic!("expected OAuthLoginFlow, got ApiRequestThen")
            }
            ToolResult::WriteFiles { .. } => {
                panic!("expected OAuthLoginFlow, got WriteFiles")
            }
        }
    }

    #[test]
    fn auth_status_validate_false_returns_immediate() {
        let auth = basic_ready();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(false));

        let result = call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));
    }

    #[test]
    fn auth_status_validate_absent_returns_immediate() {
        let auth = basic_ready();
        let result = call_tool(
            "onshape_mcp_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));
    }

    #[test]
    fn auth_status_validate_true_without_spec_returns_error() {
        let auth = basic_ready();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let err = assert_immediate_err(result);
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn auth_status_validate_true_returns_api_request_then() {
        let auth = basic_ready();
        let spec = test_spec_with_session_info();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (request, _then) = assert_api_request_then(result);
        assert_eq!(request.path, "/users/sessioninfo");
    }

    #[test]
    fn auth_status_callback_200_returns_valid_with_side_effect() {
        let auth = basic_ready();
        let spec = test_spec_with_session_info();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, then) = assert_api_request_then(result);

        let (tool_result, side_effects) = then(200, r#"{"id": "user123"}"#);

        // Should return an Immediate result with status: valid.
        let call_result = assert_immediate_ok(tool_result);
        assert_eq!(call_result.is_error, Some(false));
        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "valid");

        // Should have a side effect to update validation.
        assert_eq!(side_effects.len(), 1);
        match &side_effects[0] {
            SideEffect::UpdateValidation(state) => {
                assert_eq!(state.status, crate::ValidationStatus::Valid);
                assert!(state.last_check.is_some());
            }
        }
    }

    #[test]
    fn auth_status_callback_401_returns_invalid_with_side_effect() {
        let auth = basic_ready();
        let spec = test_spec_with_session_info();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, then) = assert_api_request_then(result);

        let (tool_result, side_effects) = then(401, "Unauthorized");

        let call_result = assert_immediate_ok(tool_result);
        assert_eq!(call_result.is_error, Some(false));
        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "invalid");

        assert_eq!(side_effects.len(), 1);
        match &side_effects[0] {
            SideEffect::UpdateValidation(state) => {
                assert_eq!(state.status, crate::ValidationStatus::Invalid);
            }
        }
    }

    #[test]
    fn auth_status_callback_500_returns_no_side_effects() {
        let auth = basic_ready();
        let spec = test_spec_with_session_info();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_mcp_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, then) = assert_api_request_then(result);

        let (tool_result, side_effects) = then(500, "Internal Server Error");

        // Should still return an Immediate result, but no side effects.
        let _call_result = assert_immediate_ok(tool_result);
        assert!(side_effects.is_empty());
    }

    #[test]
    fn auth_status_validate_false_includes_cached_validation() {
        let auth = basic_ready();
        let validation = ValidationState {
            status: crate::ValidationStatus::Valid,
            last_check: Some(chrono::Utc::now()),
            message: Some("previously validated".into()),
        };
        let result = call_tool("onshape_mcp_auth_status", None, &auth, &validation, None);
        let call_result = assert_immediate_ok(result);
        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], "valid");
    }

    // --- list_resources / read_resource tool tests ---

    #[test]
    fn call_tool_list_resources_returns_all_entries() {
        let auth = not_configured();
        let result = call_tool(
            "onshape_list_resources",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let text = call_result.content[0]
            .raw
            .as_text()
            .expect("should be text content");
        assert!(
            text.text.contains("insights:shaded-views"),
            "should list shaded-views URI"
        );
        assert!(
            text.text.contains("insights:sketch-constraints"),
            "should list sketch-constraints URI"
        );
        assert!(text.text.contains("Shaded Views"), "should include title");
    }

    #[test]
    fn call_tool_read_resource_returns_content() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "uri".to_string(),
            Value::String("insights:shaded-views".to_string()),
        );
        let result = call_tool(
            "onshape_read_resource",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_immediate_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let text = call_result.content[0]
            .raw
            .as_text()
            .expect("should be text content");
        assert!(
            text.text.contains("Part Studio Shaded Views"),
            "should contain the document title"
        );
        assert!(
            text.text.contains("pixelSize"),
            "should contain pixelSize guidance"
        );
    }

    #[test]
    fn call_tool_read_resource_unknown_uri_returns_error() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "uri".to_string(),
            Value::String("nonexistent:nothing".to_string()),
        );
        let result = call_tool(
            "onshape_read_resource",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let result = assert_immediate_ok(result);
        assert_eq!(result.is_error, Some(true));
        let text = result.content.first().expect("should have content");
        let text = match text.raw {
            rmcp::model::RawContent::Text(ref t) => &t.text,
            _ => panic!("expected text content"),
        };
        assert!(
            text.contains("not found"),
            "error should mention not found: {text}",
        );
        assert!(
            text.contains("insights:shaded-views"),
            "error should list available URIs: {text}",
        );
    }

    // ====================================================================
    // Auth Login Tool Tests
    // ====================================================================

    #[test]
    fn auth_login_default_mode_returns_proxy() {
        let auth = not_configured();
        let result = call_tool(
            "onshape_mcp_auth_login",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let mode = assert_oauth_login_flow(result);
        assert!(
            matches!(mode, LoginMode::Proxy { .. }),
            "default mode should be Proxy"
        );
    }

    #[test]
    fn auth_login_explicit_proxy_mode() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("mode".to_string(), Value::String("proxy".to_string()));
        args.insert(
            "proxy_url".to_string(),
            Value::String("https://my-proxy.example.com".to_string()),
        );

        let result = call_tool(
            "onshape_mcp_auth_login",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let mode = assert_oauth_login_flow(result);
        match mode {
            LoginMode::Proxy { proxy_url } => {
                assert_eq!(proxy_url, "https://my-proxy.example.com");
            }
            LoginMode::Direct { .. } => panic!("expected Proxy, got Direct"),
        }
    }

    #[test]
    fn auth_login_direct_mode() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("mode".to_string(), Value::String("direct".to_string()));
        args.insert(
            "client_id".to_string(),
            Value::String("my-client-id".to_string()),
        );
        args.insert(
            "client_secret".to_string(),
            Value::String("my-client-secret".to_string()),
        );

        let result = call_tool(
            "onshape_mcp_auth_login",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let mode = assert_oauth_login_flow(result);
        match mode {
            LoginMode::Direct {
                client_id,
                client_secret,
            } => {
                assert_eq!(client_id, "my-client-id");
                assert_eq!(client_secret, "my-client-secret");
            }
            LoginMode::Proxy { .. } => panic!("expected Direct, got Proxy"),
        }
    }

    #[test]
    fn auth_login_direct_mode_missing_client_id() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("mode".to_string(), Value::String("direct".to_string()));
        args.insert(
            "client_secret".to_string(),
            Value::String("secret".to_string()),
        );

        let msg = assert_tool_error(call_tool(
            "onshape_mcp_auth_login",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        ));
        assert!(msg.contains("client_id"));
    }

    #[test]
    fn auth_login_direct_mode_missing_client_secret() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("mode".to_string(), Value::String("direct".to_string()));
        args.insert("client_id".to_string(), Value::String("cid".to_string()));

        let msg = assert_tool_error(call_tool(
            "onshape_mcp_auth_login",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        ));
        assert!(msg.contains("client_secret"));
    }

    #[test]
    fn auth_login_invalid_mode() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("mode".to_string(), Value::String("invalid".to_string()));

        let msg = assert_tool_error(call_tool(
            "onshape_mcp_auth_login",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        ));
        assert!(msg.contains("invalid"));
    }

    // ====================================================================
    // Screenshot Tool Tests
    // ====================================================================

    #[allow(clippy::type_complexity)]
    fn assert_write_files(result: ToolResult) -> (Vec<FileWrite>, WriteFilesFormatter) {
        match result {
            ToolResult::WriteFiles { files, format } => (files, format),
            ToolResult::Immediate(Ok(_)) => panic!("expected WriteFiles, got Immediate Ok"),
            ToolResult::Immediate(Err(e)) => {
                panic!("expected WriteFiles, got Immediate Err: {e:?}")
            }
            ToolResult::OnshapeApiRequest { .. } => {
                panic!("expected WriteFiles, got ApiRequest")
            }
            ToolResult::OnshapeApiRequestThen { .. } => {
                panic!("expected WriteFiles, got ApiRequestThen")
            }
            ToolResult::OAuthLoginFlow { .. } => {
                panic!("expected WriteFiles, got OAuthLoginFlow")
            }
        }
    }

    // --- View matrix computation tests ---

    #[test]
    fn view_matrix_front_preset_is_named() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Front,
        };
        assert_eq!(view_matrix_string(&spec), "front");
    }

    #[test]
    fn view_matrix_back_preset_is_named() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Back,
        };
        assert_eq!(view_matrix_string(&spec), "back");
    }

    #[test]
    fn view_matrix_top_preset_is_named() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Top,
        };
        assert_eq!(view_matrix_string(&spec), "top");
    }

    #[test]
    fn view_matrix_bottom_preset_is_named() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Bottom,
        };
        assert_eq!(view_matrix_string(&spec), "bottom");
    }

    #[test]
    fn view_matrix_left_preset_is_named() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Left,
        };
        assert_eq!(view_matrix_string(&spec), "left");
    }

    #[test]
    fn view_matrix_right_preset_is_named() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Right,
        };
        assert_eq!(view_matrix_string(&spec), "right");
    }

    #[test]
    fn view_matrix_isometric_is_computed() {
        let spec = ViewSpec::Preset {
            name: ViewPreset::Isometric,
        };
        let matrix = view_matrix_string(&spec);
        // Should be a 12-number comma-separated string, not a named preset.
        assert!(!matrix.chars().all(char::is_alphabetic));
        let parts: Vec<&str> = matrix.split(',').collect();
        assert_eq!(parts.len(), 12, "view matrix should have 12 numbers");
        for part in parts {
            part.parse::<f64>()
                .expect("each part should be a valid float");
        }
    }

    #[test]
    fn view_matrix_front_angles_matches_front_view() {
        // Azimuth=0, elevation=0 should produce the front view.
        // Front view: x right, z up, -y toward viewer.
        // Row 1 (view X in model): (0, 1, 0) — but let's check the math:
        //   r00 = -sin(0) = 0, r01 = cos(0) = 1, r02 = 0
        // Row 2 (view Y in model):
        //   r10 = -cos(0)*sin(0) = 0, r11 = -sin(0)*sin(0) = 0, r12 = cos(0) = 1
        // Row 3 (view Z in model):
        //   r20 = cos(0)*cos(0) = 1, r21 = sin(0)*cos(0) = 0, r22 = sin(0) = 0
        let matrix = view_matrix_from_angles(0.0, 0.0);
        let parts: Vec<f64> = matrix
            .split(',')
            .map(|s| s.parse().expect("should be a float"))
            .collect();
        assert_eq!(parts.len(), 12);

        // Row 1: view X = (0, 1, 0) in model space
        assert!((parts[0] - 0.0).abs() < 1e-10);
        assert!((parts[1] - 1.0).abs() < 1e-10);
        assert!((parts[2] - 0.0).abs() < 1e-10);
        assert!((parts[3] - 0.0).abs() < 1e-10);

        // Row 2: view Y = (0, 0, 1) in model space
        assert!((parts[4] - 0.0).abs() < 1e-10);
        assert!((parts[5] - 0.0).abs() < 1e-10);
        assert!((parts[6] - 1.0).abs() < 1e-10);
        assert!((parts[7] - 0.0).abs() < 1e-10);

        // Row 3: view Z = (1, 0, 0) in model space
        assert!((parts[8] - 1.0).abs() < 1e-10);
        assert!((parts[9] - 0.0).abs() < 1e-10);
        assert!((parts[10] - 0.0).abs() < 1e-10);
        assert!((parts[11] - 0.0).abs() < 1e-10);
    }

    #[test]
    fn view_matrix_top_angles_matches_identity() {
        // Azimuth=0, elevation=90 should correspond to looking straight down
        // (top view). The Onshape docs say the identity matrix is the top view.
        let matrix = view_matrix_from_angles(0.0, 90.0);
        let parts: Vec<f64> = matrix
            .split(',')
            .map(|s| s.parse().expect("should be a float"))
            .collect();
        assert_eq!(parts.len(), 12);

        // Row 1: view X = (0, 1, 0)
        assert!((parts[0] - 0.0).abs() < 1e-10);
        assert!((parts[1] - 1.0).abs() < 1e-10);
        assert!((parts[2] - 0.0).abs() < 1e-10);
        assert!((parts[3] - 0.0).abs() < 1e-10);

        // Row 2: view Y = (1, 0, 0) — model +X is view up when looking down
        // r10 = -cos(0)*sin(90) = -1, r11 = -sin(0)*sin(90) = 0, r12 = cos(90) = 0
        // Hmm, that's (-1, 0, 0). Let me reconsider...
        // Looking straight down, the "up" in view space is toward model -X.
        assert!((parts[4] - -1.0).abs() < 1e-10);
        assert!((parts[5] - 0.0).abs() < 1e-10);
        assert!((parts[6] - 0.0).abs() < 1e-10);
        assert!((parts[7] - 0.0).abs() < 1e-10);

        // Row 3: view Z = (0, 0, 1)
        // r20 = cos(0)*cos(90) ≈ 0, r21 = sin(0)*cos(90) ≈ 0, r22 = sin(90) = 1
        assert!((parts[8] - 0.0).abs() < 1e-10);
        assert!((parts[9] - 0.0).abs() < 1e-10);
        assert!((parts[10] - 1.0).abs() < 1e-10);
        assert!((parts[11] - 0.0).abs() < 1e-10);
    }

    #[test]
    #[allow(clippy::suboptimal_flops)]
    fn view_matrix_columns_are_orthonormal() {
        // For any angles, the 3x3 rotation part should be orthonormal.
        for (az, el) in [(0.0, 0.0), (45.0, 35.0), (90.0, 0.0), (180.0, -45.0)] {
            let matrix = view_matrix_from_angles(az, el);
            let p: Vec<f64> = matrix
                .split(',')
                .map(|s| s.parse().expect("should be a float"))
                .collect();

            // Extract columns of the 3x3 rotation.
            let col0 = [p[0], p[4], p[8]];
            let col1 = [p[1], p[5], p[9]];
            let col2 = [p[2], p[6], p[10]];

            let dot =
                |a: &[f64; 3], b: &[f64; 3]| -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] };
            let norm = |a: &[f64; 3]| -> f64 { dot(a, a).sqrt() };

            // Each column should have unit length.
            assert!(
                (norm(&col0) - 1.0).abs() < 1e-10,
                "col0 not unit at az={az}, el={el}"
            );
            assert!(
                (norm(&col1) - 1.0).abs() < 1e-10,
                "col1 not unit at az={az}, el={el}"
            );
            assert!(
                (norm(&col2) - 1.0).abs() < 1e-10,
                "col2 not unit at az={az}, el={el}"
            );

            // Columns should be orthogonal.
            assert!(
                dot(&col0, &col1).abs() < 1e-10,
                "col0·col1 != 0 at az={az}, el={el}"
            );
            assert!(
                dot(&col0, &col2).abs() < 1e-10,
                "col0·col2 != 0 at az={az}, el={el}"
            );
            assert!(
                dot(&col1, &col2).abs() < 1e-10,
                "col1·col2 != 0 at az={az}, el={el}"
            );

            // Determinant should be positive (right-handed).
            let det = col0[0] * (col1[1] * col2[2] - col1[2] * col2[1])
                - col0[1] * (col1[0] * col2[2] - col1[2] * col2[0])
                + col0[2] * (col1[0] * col2[1] - col1[1] * col2[0]);
            assert!(
                (det - 1.0).abs() < 1e-10,
                "determinant != 1 at az={az}, el={el}: {det}"
            );
        }
    }

    // --- Screenshot tool input validation tests ---

    fn screenshot_spec() -> OpenApiSpec {
        OpenApiSpec::from_json(
            r#"{
                "openapi": "3.0.1",
                "info": { "title": "Test API", "version": "1.0" },
                "servers": [{ "url": "https://cad.onshape.com/api/v1" }],
                "paths": {
                    "/partstudios/d/{did}/{wvm}/{wvmid}/e/{eid}/shadedviews": {
                        "get": {
                            "operationId": "getPartStudioShadedViews",
                            "summary": "Get shaded views",
                            "tags": ["PartStudio"],
                            "parameters": [
                                {"name":"did","in":"path","required":true,"schema":{"type":"string"}},
                                {"name":"wvm","in":"path","required":true,"schema":{"type":"string"}},
                                {"name":"wvmid","in":"path","required":true,"schema":{"type":"string"}},
                                {"name":"eid","in":"path","required":true,"schema":{"type":"string"}},
                                {"name":"viewMatrix","in":"query","schema":{"type":"string"}},
                                {"name":"outputHeight","in":"query","schema":{"type":"integer"}},
                                {"name":"outputWidth","in":"query","schema":{"type":"integer"}},
                                {"name":"pixelSize","in":"query","schema":{"type":"number"}},
                                {"name":"edges","in":"query","schema":{"type":"string"}},
                                {"name":"useAntiAliasing","in":"query","schema":{"type":"boolean"}},
                                {"name":"showAllParts","in":"query","schema":{"type":"boolean"}},
                                {"name":"includeSurfaces","in":"query","schema":{"type":"boolean"}},
                                {"name":"includeWires","in":"query","schema":{"type":"boolean"}}
                            ],
                            "responses": { "200": {} }
                        }
                    }
                },
                "components": { "schemas": {} }
            }"#,
        )
        .expect("screenshot test spec should parse")
    }

    fn screenshot_args(view_json: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("did".to_string(), Value::String("doc1".to_string()));
        args.insert("wvm".to_string(), Value::String("w".to_string()));
        args.insert("wvmid".to_string(), Value::String("ws1".to_string()));
        args.insert("eid".to_string(), Value::String("elem1".to_string()));
        let view: Value = serde_json::from_str(view_json).expect("test view JSON");
        args.insert("view".to_string(), view);
        args.insert(
            "output_path".to_string(),
            Value::String("/tmp/test-screenshot.png".to_string()),
        );
        args
    }

    #[test]
    fn list_tools_includes_screenshot() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_screenshot"));
    }

    #[test]
    fn screenshot_invalid_edges_returns_error() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert("edges".to_string(), Value::String("invisible".to_string()));

        let msg = assert_tool_error(call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains("invisible"));
    }

    #[test]
    fn screenshot_invalid_wvm_returns_error() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert("wvm".to_string(), Value::String("x".to_string()));

        let msg = assert_tool_error(call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains("\"x\""));
    }

    #[test]
    fn screenshot_builds_api_request_then() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);

        let result = call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (request, _then) = assert_api_request_then(result);
        assert!(request.path.contains("/shadedviews"));

        // Should have pixelSize=0.
        let pixel_size = request.query_params.iter().find(|(k, _)| k == "pixelSize");
        assert_eq!(
            pixel_size.map(|(_, v)| v.as_str()),
            Some("0"),
            "pixelSize should always be 0"
        );

        // Should have viewMatrix=front.
        let view_matrix = request.query_params.iter().find(|(k, _)| k == "viewMatrix");
        assert_eq!(view_matrix.map(|(_, v)| v.as_str()), Some("front"));
    }

    #[test]
    fn screenshot_callback_api_error_returns_immediate() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);

        let result = call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, then) = assert_api_request_then(result);
        let (tool_result, side_effects) = then(500, "Internal Server Error");
        assert!(side_effects.is_empty());
        let call_result = assert_immediate_ok(tool_result);
        assert_eq!(call_result.is_error, Some(true));
    }

    #[test]
    fn screenshot_callback_success_returns_write_files() {
        let engine = base64::engine::general_purpose::STANDARD;

        let auth = not_configured();
        let spec = screenshot_spec();
        let args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);

        let result = call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, then) = assert_api_request_then(result);

        // Simulate a successful API response with a base64-encoded PNG.
        let fake_png = b"fake png data";
        let encoded = engine.encode(fake_png);
        let body = serde_json::json!({ "images": [encoded] }).to_string();

        let (tool_result, side_effects) = then(200, &body);
        assert!(side_effects.is_empty());

        let (files, _format) = assert_write_files(tool_result);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].data, fake_png);
        assert_eq!(
            files[0].path,
            std::path::PathBuf::from("/tmp/test-screenshot.png")
        );
    }

    #[test]
    fn screenshot_format_result_success_includes_view_matrix() {
        let result = FileWriteResult::Success {
            path: std::path::PathBuf::from("/tmp/screenshot.png"),
        };

        let call_result = format_screenshot_result(&result, "front", "front");
        assert_eq!(call_result.is_error, Some(false));
        assert_eq!(call_result.content.len(), 2);

        // JSON content should include view_matrix.
        let json_text = call_result.content[0]
            .as_text()
            .expect("first content should be text");
        let json: Value = serde_json::from_str(&json_text.text).expect("should be valid JSON");
        assert_eq!(json["view_matrix"], "front");
        assert_eq!(json["status"], "ok");

        // Human-readable content should include viewMatrix.
        let text = call_result.content[1]
            .as_text()
            .expect("second content should be text");
        assert!(text.text.contains("viewMatrix=front"));
        assert!(text.text.contains("/tmp/screenshot.png"));
    }

    #[test]
    fn screenshot_format_result_failure() {
        let result = FileWriteResult::Error {
            path: std::path::PathBuf::from("/tmp/screenshot.png"),
            message: "Permission denied".to_string(),
        };

        let call_result = format_screenshot_result(&result, "top", "top");
        assert_eq!(call_result.is_error, Some(true));

        let json_text = call_result.content[0]
            .as_text()
            .expect("first content should be text");
        let json: Value = serde_json::from_str(&json_text.text).expect("should be valid JSON");
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"], "Permission denied");

        let text = call_result.content[1]
            .as_text()
            .expect("second content should be text");
        assert!(text.text.contains("FAILED"));
        assert!(text.text.contains("Permission denied"));
    }

    #[test]
    fn screenshot_optional_params_passed_to_request() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert("output_height".to_string(), Value::Number(1000.into()));
        args.insert("output_width".to_string(), Value::Number(800.into()));
        args.insert("edges".to_string(), Value::String("hide".to_string()));
        args.insert("use_anti_aliasing".to_string(), Value::Bool(true));
        args.insert("show_all_parts".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (request, _then) = assert_api_request_then(result);

        let find_param = |name: &str| -> Option<String> {
            request
                .query_params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(find_param("outputHeight"), Some("1000".to_string()));
        assert_eq!(find_param("outputWidth"), Some("800".to_string()));
        assert_eq!(find_param("edges"), Some("hide".to_string()));
        assert_eq!(find_param("useAntiAliasing"), Some("true".to_string()));
        assert_eq!(find_param("showAllParts"), Some("true".to_string()));
    }

    #[test]
    fn screenshot_custom_output_path() {
        let engine = base64::engine::general_purpose::STANDARD;

        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert(
            "output_path".to_string(),
            Value::String("/home/user/my-part.png".to_string()),
        );

        let result = call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, then) = assert_api_request_then(result);

        let body = serde_json::json!({ "images": [engine.encode(b"data")] }).to_string();
        let (tool_result, _) = then(200, &body);
        let (files, _format) = assert_write_files(tool_result);
        assert_eq!(
            files[0].path,
            std::path::PathBuf::from("/home/user/my-part.png")
        );
    }

    #[test]
    fn view_spec_preset_deserializes() {
        let json = r#"{"type": "preset", "name": "isometric"}"#;
        let spec: ViewSpec = serde_json::from_str(json).expect("should deserialize");
        match spec {
            ViewSpec::Preset { name } => assert_eq!(name, ViewPreset::Isometric),
            ViewSpec::Angles { .. } => panic!("expected Preset"),
        }
    }

    #[test]
    fn view_spec_angles_deserializes() {
        let json = r#"{"type": "angles", "azimuth": 45.0, "elevation": 30.0}"#;
        let spec: ViewSpec = serde_json::from_str(json).expect("should deserialize");
        match spec {
            ViewSpec::Angles { azimuth, elevation } => {
                assert!((azimuth - 45.0).abs() < f64::EPSILON);
                assert!((elevation - 30.0).abs() < f64::EPSILON);
            }
            ViewSpec::Preset { .. } => panic!("expected Angles"),
        }
    }
}
