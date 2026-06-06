//! Tool definitions and dispatch logic.
//!
//! This module contains all tool metadata and pure business logic for tool execution.
//! Uses rmcp types directly to avoid unnecessary type conversions.
//!
//! ## Effect Pattern
//!
//! Tool dispatch returns a [`ToolEffect`] which is either:
//! - `Done` — the tool completed with no I/O needed
//! - `ApiRequest` — the tool needs an HTTP request executed by the I/O layer
//!
//! Multi-step operations use a [`Continuation`] enum (plain data) instead of
//! closures. After the I/O layer executes an effect, it calls [`resume()`] with
//! the continuation and an [`IoResult`] to get the next effect.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use http::{HeaderMap, HeaderName, HeaderValue};

use rmcp::{
    ErrorData,
    model::{CallToolResult, Content, ErrorCode, Tool, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use onshape_client_core::request::{ApiRequest, BinaryField, RequestBody};
use onshape_openapi::{OpenApiSpec, SearchFilters};

use crate::config::ResolvedAuth;
use crate::{AuthStatusResult, ValidationState};

// ============================================================================
// Effect Type
// ============================================================================

/// Side effects that [`resume()`] can request the I/O layer to apply.
///
/// Returned alongside a [`ToolEffect`] from [`resume()`]. The I/O layer is
/// responsible for applying these effects after processing the result.
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
#[derive(Debug)]
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

// ============================================================================
// File Read Effect Types
// ============================================================================

/// A file to be read from disk by the I/O layer.
///
/// This is a pure data description of the read — no I/O is performed here.
#[derive(Debug)]
pub struct FileRead {
    /// The file path to read.
    pub path: PathBuf,
}

/// The outcome of a single file read attempt, reported by the I/O layer.
pub enum FileReadResult {
    /// The file was read successfully.
    Success {
        /// The path that was read.
        path: PathBuf,
        /// The raw file contents.
        data: Vec<u8>,
    },
    /// The file read failed.
    Error {
        /// The path that was attempted.
        path: PathBuf,
        /// Human-readable error message.
        message: String,
    },
}

/// How the OAuth login flow should operate.
///
/// Returned as part of [`ToolEffect::OAuthLoginFlow`] for the I/O layer
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
/// Tools that need no I/O return `Done`. Tools that need an HTTP request
/// to the Onshape API return `ApiRequest` with a [`Continuation`] that
/// describes how to process the response. The I/O layer executes the
/// effect and calls [`resume()`] with the continuation and response to
/// get the next effect.
///
/// All variants are plain data — no closures — so `ToolEffect` is
/// inspectable, `Debug`-printable, and testable.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ToolEffect {
    /// Tool completed — no further I/O needed.
    Done(Result<CallToolResult, ErrorData>),
    /// Tool needs an HTTP request to the Onshape API.
    ///
    /// After executing the request, the I/O layer calls [`resume()`] with
    /// the continuation and an [`IoResult::ApiResponse`].
    ApiRequest {
        /// The HTTP request to execute.
        request: ApiRequest,
        /// What to do with the response.
        continuation: Continuation,
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
    /// [`FileWriteResult`]. Then it calls [`resume()`] with the continuation
    /// and an [`IoResult::FileWriteResults`] to get the next effect.
    WriteFiles {
        /// Files to write.
        files: Vec<FileWrite>,
        /// What to do with the write results.
        continuation: Continuation,
    },
    /// Tool needs files read from disk.
    ///
    /// The I/O layer reads each [`FileRead`] and reports outcomes via
    /// [`FileReadResult`]. Then it calls [`resume()`] with the continuation
    /// and an [`IoResult::FileReadResults`] to get the next effect.
    ReadFiles {
        /// Files to read.
        reads: Vec<FileRead>,
        /// What to do with the read results.
        continuation: Continuation,
    },
}

/// Plain-data continuation describing how to process an I/O result.
///
/// Each variant corresponds to a specific tool's post-I/O processing logic.
/// The [`resume()`] function dispatches on these variants to produce the
/// next [`ToolEffect`].
#[derive(Debug)]
pub enum Continuation {
    /// After API call: format the response as the final tool result.
    ///
    /// Used by `onshape_api_call`.
    FormatApiResponse,
    /// After API call: process auth validation response.
    ///
    /// Used by `onshape_auth_status` with `validate=true`.
    ProcessAuthValidation {
        /// The resolved auth state at the time the validation was requested.
        resolved_auth: ResolvedAuth,
    },
    /// After API call: decode screenshot image, produce `WriteFiles` effect.
    ///
    /// Used by `onshape_screenshot`.
    ProcessScreenshotResponse {
        /// Where to write the screenshot file.
        output_path: PathBuf,
        /// Human-readable view label (e.g. "front", "isometric").
        label: String,
        /// The view matrix string used in the API request.
        view_matrix: String,
    },
    /// After file writes: format the screenshot write result.
    ///
    /// Used as the continuation in the `WriteFiles` effect produced by
    /// [`Continuation::ProcessScreenshotResponse`].
    FormatScreenshotWrite {
        /// Human-readable view label.
        label: String,
        /// The view matrix string used in the API request.
        view_matrix: String,
    },
    /// After file reads: inject file content into an already-built API request.
    ///
    /// Used by `onshape_api_call` when `file_refs` are present. The request
    /// was built by [`OpenApiSpec::build_request()`] with file-ref fields
    /// omitted from the body. After reading files, [`resume()`] injects
    /// the content into the request body and returns an [`ToolEffect::ApiRequest`].
    InjectFilesIntoRequest {
        /// The pre-built API request (file-ref fields absent from the body).
        request: ApiRequest,
        /// File references describing which fields to populate.
        file_refs: Vec<FileReference>,
    },
}

/// What the I/O layer feeds back to [`resume()`] after executing an effect.
pub enum IoResult<'a> {
    /// Response from an HTTP API request.
    ApiResponse {
        /// HTTP status code.
        status: u16,
        /// Response headers as `(name, value)` pairs.
        headers: &'a [(String, String)],
        /// Raw response body bytes.
        body: &'a [u8],
    },
    /// Results of file write operations.
    FileWriteResults(&'a [FileWriteResult]),
    /// Results of file read operations.
    FileReadResults(&'a [FileReadResult]),
}

/// Create a [`ToolEffect`] for an expected user-input error.
///
/// Returns a successful `CallToolResult` with `is_error: Some(true)`, keeping
/// the MCP transport clean. Use this for validation failures that the caller
/// (typically an LLM) can act on — as opposed to protocol-level
/// `Err(ErrorData)` which signals handler/infrastructure breakage.
fn tool_input_error(message: impl Into<String>) -> ToolEffect {
    ToolEffect::Done(Ok(CallToolResult::error(vec![Content::text(
        message.into(),
    )])))
}

/// Validate a file path for use in file I/O effects.
///
/// Returns `Ok(PathBuf)` if the path is valid. Returns `Err(message)` if:
/// - The path is empty or whitespace-only
/// - The path has no file name component
/// - The path contains `..` segments (directory traversal)
fn validate_file_path(path: &str) -> Result<PathBuf, String> {
    let path_buf = PathBuf::from(path);
    if path.trim().is_empty() || path_buf.file_name().is_none() {
        return Err(format!("file path must include a file name: {path:?}"));
    }
    if path_buf
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "file path must not contain '..' segments: {path:?}"
        ));
    }
    Ok(path_buf)
}

fn header_params_to_header_map(params: &HashMap<String, String>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, value) in params {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("invalid header name {name:?}: {e}"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|e| format!("invalid value for header {name:?}: {e}"))?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

/// Convert a raw HTTP response from the Onshape API into a [`CallToolResult`].
///
/// # Arguments
///
/// * `status` - HTTP status code
/// * `headers` - Response headers as `(name, value)` pairs
/// * `body` - Raw response body bytes
///
/// # Errors
///
/// Returns an error if the response cannot be processed.
pub fn process_api_response(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<CallToolResult, ErrorData> {
    let is_success = (200..300).contains(&status);
    let body_text = match std::str::from_utf8(body) {
        Ok(text) if response_content_type(headers).is_none_or(content_type_is_textual) => text,
        _ => {
            let content = binary_api_response_content(status, headers, body, !is_success)?;
            return Ok(if is_success {
                CallToolResult::success(vec![content])
            } else {
                CallToolResult::error(vec![content])
            });
        }
    };

    if is_success {
        // Try to parse as JSON for nice formatting
        let content = if let Ok(json_val) = serde_json::from_str::<Value>(body_text) {
            Content::json(&json_val).map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("failed to serialize API response: {e}"),
                    None,
                )
            })?
        } else {
            Content::text(body_text)
        };

        Ok(CallToolResult::success(vec![content]))
    } else {
        let content = Content::text(format!("API error (HTTP {status}): {body_text}"));
        Ok(CallToolResult::error(vec![content]))
    }
}

fn response_content_type(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
}

fn content_type_is_textual(content_type: &str) -> bool {
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();

    media_type.starts_with("text/")
        || media_type == "application/json"
        || media_type.ends_with("+json")
        || media_type == "application/xml"
        || media_type.ends_with("+xml")
        || media_type == "application/javascript"
        || media_type == "application/x-www-form-urlencoded"
}

fn binary_api_response_content(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    is_error: bool,
) -> Result<Content, ErrorData> {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut response = serde_json::json!({
        "status": status,
        "encoding": "base64",
        "byteLength": body.len(),
        "body": engine.encode(body),
    });

    if let Some(content_type) = response_content_type(headers)
        && let Some(response) = response.as_object_mut()
    {
        response.insert(
            "contentType".to_string(),
            Value::String(content_type.to_string()),
        );
    }

    if is_error && let Some(response) = response.as_object_mut() {
        response.insert(
            "error".to_string(),
            Value::String(format!("API error (HTTP {status})")),
        );
    }

    Content::json(&response).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize binary API response metadata: {e}"),
            None,
        )
    })
}

/// Resume a multi-step tool operation after the I/O layer has executed an effect.
///
/// This is a pure function: given a [`Continuation`] and the [`IoResult`] from
/// the I/O layer, it produces the next [`ToolEffect`] plus any [`SideEffect`]s
/// for the I/O layer to apply.
///
/// # Panics
///
/// Panics if `continuation` and `result` are mismatched (e.g., an API
/// continuation paired with file-write results). This indicates a programming
/// error in the I/O layer.
#[must_use]
pub fn resume(continuation: Continuation, result: IoResult<'_>) -> (ToolEffect, Vec<SideEffect>) {
    match (continuation, result) {
        // --- FormatApiResponse ---
        (
            Continuation::FormatApiResponse,
            IoResult::ApiResponse {
                status,
                headers,
                body,
            },
        ) => (
            ToolEffect::Done(process_api_response(status, headers, body)),
            vec![],
        ),

        // --- ProcessAuthValidation ---
        (
            Continuation::ProcessAuthValidation { resolved_auth },
            IoResult::ApiResponse {
                status,
                headers: _,
                body: _,
            },
        ) => resume_auth_validation(status, &resolved_auth),

        // --- ProcessScreenshotResponse ---
        (
            Continuation::ProcessScreenshotResponse {
                output_path,
                label,
                view_matrix,
            },
            IoResult::ApiResponse {
                status,
                headers: _,
                body,
            },
        ) => resume_screenshot_response(status, body, output_path, label, view_matrix),

        // --- FormatScreenshotWrite ---
        (
            Continuation::FormatScreenshotWrite { label, view_matrix },
            IoResult::FileWriteResults(results),
        ) => {
            let Some(result) = results.first() else {
                return (
                    ToolEffect::Done(Ok(CallToolResult::error(vec![Content::text(
                        "internal error: no file write results",
                    )]))),
                    vec![],
                );
            };
            (
                ToolEffect::Done(Ok(format_screenshot_result(result, &label, &view_matrix))),
                vec![],
            )
        }

        // --- InjectFilesIntoRequest ---
        (
            Continuation::InjectFilesIntoRequest { request, file_refs },
            IoResult::FileReadResults(results),
        ) => (resume_inject_files(request, &file_refs, results), vec![]),

        // --- Mismatched continuation/result ---
        (continuation, result) => {
            unreachable!(
                "mismatched Continuation and IoResult: continuation={continuation:?}, \
                 result kind={}",
                match result {
                    IoResult::ApiResponse { .. } => "ApiResponse",
                    IoResult::FileWriteResults(_) => "FileWriteResults",
                    IoResult::FileReadResults(_) => "FileReadResults",
                }
            )
        }
    }
}

/// Inject file content into an API request body after file reads complete.
///
/// Extracted from [`resume()`] for readability.
///
/// Handles both JSON and multipart request bodies:
///
/// - **JSON**: injects content as string values at top-level keys.
///   [`FileEncoding::TextUtf8`] produces a UTF-8 string, [`FileEncoding::Base64`]
///   produces a base64-encoded string, and [`FileEncoding::RawBytes`] is an error.
/// - **Multipart**: [`FileEncoding::RawBytes`] adds a binary form field,
///   [`FileEncoding::TextUtf8`] adds a text form field, and
///   [`FileEncoding::Base64`] adds a text form field with base64 content.
#[allow(clippy::too_many_lines)]
fn resume_inject_files(
    mut request: ApiRequest,
    file_refs: &[FileReference],
    results: &[FileReadResult],
) -> ToolEffect {
    // Build a map from path → data for successful reads.
    let mut reads: HashMap<PathBuf, &[u8]> = HashMap::new();
    for result in results {
        match result {
            FileReadResult::Success { path, data } => {
                reads.insert(path.clone(), data);
            }
            FileReadResult::Error { path, message } => {
                return tool_input_error(format!(
                    "failed to read file {}: {message}",
                    path.display()
                ));
            }
        }
    }

    let Some(body) = request.body.as_mut() else {
        return tool_input_error("file_refs provided but the endpoint has no request body");
    };

    match body {
        RequestBody::Json(value) => {
            let Some(obj) = value.as_object_mut() else {
                return tool_input_error("file_refs require the request body to be a JSON object");
            };

            for file_ref in file_refs {
                if let Err(e) = inject_into_json_field(obj, file_ref, &reads) {
                    return tool_input_error(e);
                }
            }
        }

        RequestBody::Multipart(multipart) => {
            for file_ref in file_refs {
                if let Err(e) = inject_into_multipart_field(multipart, file_ref, &reads) {
                    return tool_input_error(e);
                }
            }
        }
    }

    ToolEffect::ApiRequest {
        request,
        continuation: Continuation::FormatApiResponse,
    }
}

/// Inject a single file reference into a JSON object field.
///
/// Returns `Err(message)` on encoding/lookup errors.
fn inject_into_json_field(
    obj: &mut Map<String, Value>,
    file_ref: &FileReference,
    reads: &HashMap<PathBuf, &[u8]>,
) -> Result<(), String> {
    let path = PathBuf::from(&file_ref.path);
    let Some(data) = reads.get(&path) else {
        return Err(format!(
            "no read result for file reference: {}",
            file_ref.path
        ));
    };

    match file_ref.encoding {
        FileEncoding::TextUtf8 => {
            let text = std::str::from_utf8(data)
                .map_err(|e| format!("file {} is not valid UTF-8: {e}", file_ref.path))?;
            obj.insert(file_ref.field.clone(), Value::String(text.to_owned()));
        }
        FileEncoding::Base64 => {
            let engine = base64::engine::general_purpose::STANDARD;
            let encoded = engine.encode(data);
            obj.insert(file_ref.field.clone(), Value::String(encoded));
        }
        FileEncoding::RawBytes => {
            return Err(format!(
                "file_ref for field {:?} uses raw_bytes encoding, which cannot \
                 be used with JSON request bodies. Use text_utf8 or base64 instead.",
                file_ref.field
            ));
        }
    }
    Ok(())
}

/// Inject a single file reference into a multipart form body.
///
/// Returns `Err(message)` on encoding/lookup errors.
fn inject_into_multipart_field(
    multipart: &mut onshape_client_core::request::MultipartBody,
    file_ref: &FileReference,
    reads: &HashMap<PathBuf, &[u8]>,
) -> Result<(), String> {
    let path = PathBuf::from(&file_ref.path);
    let Some(data) = reads.get(&path) else {
        return Err(format!(
            "no read result for file reference: {}",
            file_ref.path
        ));
    };

    // Strip any existing entries for this field so file_ref wins
    // (matches JSON path semantics where Map::insert replaces).
    multipart
        .text_fields
        .retain(|(name, _)| name != &file_ref.field);
    multipart
        .binary_fields
        .retain(|f| f.field_name != file_ref.field);

    match file_ref.encoding {
        FileEncoding::RawBytes => {
            multipart.binary_fields.push(BinaryField {
                field_name: file_ref.field.clone(),
                data: data.to_vec(),
                content_type: None,
            });
        }
        FileEncoding::TextUtf8 => {
            let text = std::str::from_utf8(data)
                .map_err(|e| format!("file {} is not valid UTF-8: {e}", file_ref.path))?;
            multipart
                .text_fields
                .push((file_ref.field.clone(), text.to_owned()));
        }
        FileEncoding::Base64 => {
            let engine = base64::engine::general_purpose::STANDARD;
            let encoded = engine.encode(data);
            multipart
                .text_fields
                .push((file_ref.field.clone(), encoded));
        }
    }
    Ok(())
}

/// Process auth validation API response.
///
/// Extracted from [`resume()`] for readability.
fn resume_auth_validation(
    status: u16,
    resolved_auth: &ResolvedAuth,
) -> (ToolEffect, Vec<SideEffect>) {
    let now = chrono::Utc::now();

    if (200..300).contains(&status) {
        let valid_state = ValidationState {
            status: crate::ValidationStatus::Valid,
            last_check: Some(now),
            message: Some("Credentials validated successfully".into()),
        };
        let result = AuthStatusResult::new(resolved_auth, Some(&valid_state), now);
        let tool_effect = match Content::json(&result) {
            Ok(c) => ToolEffect::Done(Ok(CallToolResult::success(vec![c]))),
            Err(e) => ToolEffect::Done(Err(e)),
        };
        (tool_effect, vec![SideEffect::UpdateValidation(valid_state)])
    } else if status == 401 {
        let invalid_state = ValidationState {
            status: crate::ValidationStatus::Invalid,
            last_check: Some(now),
            message: Some("API returned 401 Unauthorized — credentials are invalid".into()),
        };
        let result = AuthStatusResult::new(resolved_auth, Some(&invalid_state), now);
        let tool_effect = match Content::json(&result) {
            Ok(c) => ToolEffect::Done(Ok(CallToolResult::success(vec![c]))),
            Err(e) => ToolEffect::Done(Err(e)),
        };
        (
            tool_effect,
            vec![SideEffect::UpdateValidation(invalid_state)],
        )
    } else {
        // Unexpected status — don't update validation state.
        let result = AuthStatusResult::new(resolved_auth, None, now);
        let mut auth_result = match Content::json(&result) {
            Ok(c) => CallToolResult::success(vec![c]),
            Err(e) => {
                return (ToolEffect::Done(Err(e)), vec![]);
            }
        };
        // Add a note about the unexpected status.
        auth_result.content.push(Content::text(format!(
            "Warning: credential validation returned unexpected HTTP {status}"
        )));
        (ToolEffect::Done(Ok(auth_result)), vec![])
    }
}

/// Process screenshot API response: decode base64 image and produce `WriteFiles`.
///
/// Extracted from [`resume()`] for readability.
fn resume_screenshot_response(
    status: u16,
    body: &[u8],
    output_path: PathBuf,
    label: String,
    view_matrix: String,
) -> (ToolEffect, Vec<SideEffect>) {
    if !(200..300).contains(&status) {
        let body = String::from_utf8_lossy(body);
        return (
            ToolEffect::Done(Ok(CallToolResult::error(vec![Content::text(format!(
                "Shaded views API error (HTTP {status}): {body}"
            ))]))),
            vec![],
        );
    }

    // Parse the response JSON.
    let response: Value = match serde_json::from_slice(body) {
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
        ToolEffect::WriteFiles {
            files: vec![file_write],
            continuation: Continuation::FormatScreenshotWrite { label, view_matrix },
        },
        vec![],
    )
}

// ============================================================================
// Tool Registry
// ============================================================================

/// Returns all available tools.
#[must_use]
pub fn list_tools() -> Vec<Tool> {
    vec![
        tool_get_started_def(),
        tool_auth_status_def(),
        tool_auth_login_def(),
        tool_api_search_def(),
        tool_api_explain_def(),
        tool_api_call_def(),
        tool_api_schema_def(),
        tool_list_resources_def(),
        tool_read_resource_def(),
        tool_screenshot_def(),
        tool_error_lookup_def(),
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
/// Returns an error (via `ToolEffect::Done`) if the tool is not found
/// or input validation fails. Returns `ToolEffect::ApiRequest` if the
/// tool needs an HTTP request executed.
#[must_use]
pub fn call_tool(
    name: &str,
    arguments: Option<&Map<String, Value>>,
    resolved_auth: &ResolvedAuth,
    validation: &ValidationState,
    spec: Option<&OpenApiSpec>,
) -> ToolEffect {
    match name {
        "onshape_mcp_get_started" => ToolEffect::Done(Ok(call_get_started())),
        "onshape_auth_status" => call_auth_status(arguments, resolved_auth, validation, spec),
        "onshape_auth_login" => call_auth_login(arguments),
        "onshape_api_search" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolEffect::Done(Err(e)),
            };
            ToolEffect::Done(call_api_search(arguments, spec))
        }
        "onshape_api_explain" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolEffect::Done(Err(e)),
            };
            ToolEffect::Done(call_api_explain(arguments, spec))
        }
        "onshape_api_call" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolEffect::Done(Err(e)),
            };
            call_api_call(arguments, spec)
        }
        "onshape_api_schema" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolEffect::Done(Err(e)),
            };
            ToolEffect::Done(call_api_schema(arguments, spec))
        }
        "onshape_list_resources" => ToolEffect::Done(Ok(call_list_resources())),
        "onshape_read_resource" => ToolEffect::Done(Ok(call_read_resource(arguments))),
        "onshape_error_lookup" => ToolEffect::Done(Ok(call_error_lookup(arguments))),
        "onshape_screenshot" => {
            let spec = match require_spec(spec) {
                Ok(s) => s,
                Err(e) => return ToolEffect::Done(Err(e)),
            };
            call_screenshot(arguments, spec)
        }
        _ => ToolEffect::Done(Err(ErrorData::new(
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

/// Input schema for `onshape_auth_status`.
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

/// How file content should be encoded when injected into a request body.
///
/// The encoding determines how raw file bytes are converted for the target
/// field. The appropriate encoding depends on the field type:
///
/// - **Text fields** (JSON string values, multipart text parts): use [`TextUtf8`](FileEncoding::TextUtf8)
/// - **Binary fields** (multipart `format: binary` parts): use [`RawBytes`](FileEncoding::RawBytes)
/// - **Embedded binary in JSON**: use [`Base64`](FileEncoding::Base64)
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileEncoding {
    /// Read the file as UTF-8 text.
    ///
    /// For JSON bodies: injected as a JSON string value.
    /// For multipart bodies: injected as a text form field.
    ///
    /// Returns an error if the file is not valid UTF-8.
    TextUtf8,
    /// Read the file as raw bytes and base64-encode.
    ///
    /// For JSON bodies: injected as a base64-encoded JSON string value.
    /// For multipart bodies: injected as a text form field containing base64.
    Base64,
    /// Read the file as raw bytes (no encoding).
    ///
    /// For multipart bodies: injected as a binary form field (`BinaryField`).
    /// For JSON bodies: returns an error (raw bytes cannot be embedded in JSON).
    RawBytes,
}

/// A reference to a file whose content should be injected into a request body field.
///
/// Instead of the LLM reading a file into its context and inlining the content,
/// the server reads the file at execution time. This keeps large file content
/// out of the LLM's context window.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FileReference {
    /// File system path to read. Must not be empty or contain `..` segments.
    pub path: String,
    /// The body field name to populate with the file content.
    ///
    /// For JSON bodies, this is a top-level key in the JSON object.
    /// For multipart bodies, this is the form field name.
    pub field: String,
    /// How to encode the file content for the target field.
    pub encoding: FileEncoding,
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
    /// Header parameters (e.g., `{"Accept": "application/octet-stream"}`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub header_params: HashMap<String, String>,
    /// JSON string for the request body (for POST/PUT/PATCH endpoints).
    /// Use `onshape_api_explain` to see the expected schema for each endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// File references for fields whose content should be read from disk.
    ///
    /// Each reference specifies a file path, a body field name, and an encoding.
    /// The server reads the files and injects their content into the request body
    /// after building the request. Fields listed here should be omitted from `body`.
    ///
    /// Example: to upload a file via `uploadFileCreateElement`, pass the metadata
    /// fields in `body` and use `file_refs` for the binary content:
    /// `body: "{\"formatName\": \"PARASOLID\"}"`,
    /// `file_refs: [{"path": "/tmp/part.x_t", "field": "file", "encoding": "raw_bytes"}]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_refs: Vec<FileReference>,
}

/// Input schema for `onshape_auth_login`.
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

/// Input schema for `onshape_api_schema`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ApiSchemaInput {
    /// The schema name to look up (e.g., `"BTMParameterEnum-145"` or
    /// `"BTFeatureDefinitionCall-1406"`). Use schema names from
    /// `x-bttype-options` annotations in `onshape_api_explain` results,
    /// or from the `subtypes` field in previous `onshape_api_schema` results.
    pub schema: String,
}

/// Input schema for `onshape_read_resource`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadResourceInput {
    /// The URI of the resource to read (e.g., `"insights:shaded-views"`).
    /// Use `onshape_list_resources` to discover available URIs.
    pub uri: String,
}

/// Input schema for `onshape_error_lookup`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ErrorLookupInput {
    /// One or more `ErrorStringEnum` values to resolve (e.g.,
    /// `["FILLET_FAILED", "SWEEP_PATH_FAILED"]`).
    pub values: Vec<String>,
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

#[allow(clippy::expect_used)]
fn tool_get_started_def() -> Tool {
    let schema = schemars::schema_for!(EmptyInput);
    let input_schema: Value =
        serde_json::to_value(schema).expect("EmptyInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_mcp_get_started",
        "Get essential guidance for working with this server. Call this \
         before starting any Onshape task — it explains how the server's \
         tools and resources fit together and how to avoid common mistakes.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

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
        "onshape_auth_status",
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
        "onshape_auth_login",
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
         {\"did\": \"abc123\"}), not baked into a URL string. For endpoints that accept \
         file content (e.g., file uploads), use `file_refs` to reference local files \
         instead of inlining content in the body — the server reads them directly. \
         Returns the API response.",
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
         These cover tested patterns for common workflows like creating features, \
         handling errors, working with sketches, and more. A good first step when \
         starting a new task, encountering errors, or unsure how to approach \
         something — the answer may already be documented here. Returns URIs, \
         titles, and descriptions. Use onshape_read_resource to read a specific \
         resource by URI.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

#[allow(clippy::expect_used)]
fn tool_api_schema_def() -> Tool {
    let schema = schemars::schema_for!(ApiSchemaInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("ApiSchemaInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_api_schema",
        "Look up an Onshape API schema by name. Returns the schema's properties \
         (merged with inherited parent properties), discriminator subtypes if \
         polymorphic, and parent type. Use schema names from x-bttype-options \
         annotations in onshape_api_explain results to drill into specific types.",
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
        "Read a specific resource document by URI. Returns the full markdown \
         content with tested patterns, working examples, and practical guidance \
         for Onshape API usage. Worth checking before resorting to trial and \
         error with the API — these documents can save significant time. Use \
         onshape_list_resources to discover available URIs.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

#[allow(clippy::expect_used)]
fn tool_error_lookup_def() -> Tool {
    let schema = schemars::schema_for!(ErrorLookupInput);
    let input_schema: Value = serde_json::to_value(schema)
        .expect("ErrorLookupInput schema serialization should never fail");
    let input_schema = input_schema
        .as_object()
        .cloned()
        .expect("Schema should be a JSON object");

    Tool::new(
        "onshape_error_lookup",
        "Resolve FeatureScript ErrorStringEnum values to human-readable messages. \
         Use this when a feature's statusEnum is not CUSTOM_ERROR and no statusMsg \
         is available. Accepts one or more enum names and returns their messages.",
        Arc::new(input_schema),
    )
    .annotate(ToolAnnotations::new().read_only(true).destructive(false))
}

// ============================================================================
// Tool Implementations
// ============================================================================

fn call_get_started() -> CallToolResult {
    CallToolResult::success(vec![Content::text(crate::instructions())])
}

fn call_auth_status(
    arguments: Option<&Map<String, Value>>,
    resolved_auth: &ResolvedAuth,
    validation: &ValidationState,
    spec: Option<&OpenApiSpec>,
) -> ToolEffect {
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
            Err(e) => return ToolEffect::Done(Err(e)),
        };
        return ToolEffect::Done(Ok(CallToolResult::success(vec![content])));
    }

    // validate=true: actively check credentials via GET /users/sessioninfo.
    let spec = match require_spec(spec) {
        Ok(s) => s,
        Err(e) => return ToolEffect::Done(Err(e)),
    };

    let empty_map = HashMap::new();
    let empty_headers = HeaderMap::new();
    let request =
        match spec.build_request("sessionInfo", &empty_map, &empty_map, &empty_headers, None) {
            Ok(req) => req,
            Err(e) => {
                return ToolEffect::Done(Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("failed to build sessionInfo request: {e}"),
                    None,
                )));
            }
        };

    ToolEffect::ApiRequest {
        request,
        continuation: Continuation::ProcessAuthValidation {
            resolved_auth: resolved_auth.clone(),
        },
    }
}

/// Default OAuth proxy URL used when no `proxy_url` is specified.
pub const DEFAULT_PROXY_URL: &str = "https://onshape-oauth-proxy.fstab.workers.dev";

fn call_auth_login(arguments: Option<&Map<String, Value>>) -> ToolEffect {
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

    ToolEffect::OAuthLoginFlow { mode }
}

fn call_api_search(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiSearchInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.message)])),
    };
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

    Ok(CallToolResult::success(vec![content]))
}

fn call_api_explain(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiExplainInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.message)])),
    };
    let detail = match spec.explain(&input.endpoint) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!("{e}"))]));
        }
    };

    let content = Content::json(&detail).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize endpoint detail: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult::success(vec![content]))
}

fn call_api_call(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolEffect {
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

    // Validate file reference paths and field names before building the request.
    for file_ref in &input.file_refs {
        if let Err(msg) = validate_file_path(&file_ref.path) {
            return tool_input_error(format!("invalid file_ref path: {msg}"));
        }
        if file_ref.field.trim().is_empty() {
            return tool_input_error("invalid file_ref field: field must not be empty");
        }
    }

    let header_params = match header_params_to_header_map(&input.header_params) {
        Ok(headers) => headers,
        Err(msg) => return tool_input_error(msg),
    };

    let request = match spec.build_request(
        &input.endpoint,
        &input.path_params,
        &input.query_params,
        &header_params,
        body,
    ) {
        Ok(req) => req,
        Err(e) => {
            return tool_input_error(format!("{e}"));
        }
    };

    // Validate request body shape before scheduling file reads.
    // resume_inject_files() rejects these cases too (defense-in-depth),
    // but checking early avoids unnecessary disk I/O.
    if !input.file_refs.is_empty() {
        match request.body.as_ref() {
            Some(RequestBody::Json(value)) => {
                if !value.is_object() {
                    return tool_input_error(
                        "file_refs require the request body to be a JSON object",
                    );
                }
                if input
                    .file_refs
                    .iter()
                    .any(|fr| matches!(fr.encoding, FileEncoding::RawBytes))
                {
                    return tool_input_error(
                        "raw_bytes file_refs cannot be used with JSON request bodies; \
                         use text_utf8 or base64 instead",
                    );
                }
            }
            Some(RequestBody::Multipart(_)) => {}
            None => {
                return tool_input_error("file_refs provided but the endpoint has no request body");
            }
        }
    }

    // If file references are present, emit a ReadFiles effect first.
    // After reads complete, resume() will inject the content and forward
    // the request as an ApiRequest effect.
    if input.file_refs.is_empty() {
        ToolEffect::ApiRequest {
            request,
            continuation: Continuation::FormatApiResponse,
        }
    } else {
        let mut seen = std::collections::HashSet::new();
        let reads: Vec<FileRead> = input
            .file_refs
            .iter()
            .filter_map(|fr| {
                let path = PathBuf::from(&fr.path);
                seen.insert(path.clone()).then_some(FileRead { path })
            })
            .collect();

        ToolEffect::ReadFiles {
            reads,
            continuation: Continuation::InjectFilesIntoRequest {
                request,
                file_refs: input.file_refs,
            },
        }
    }
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

    CallToolResult::success(vec![Content::text(output)])
}

fn call_api_schema(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiSchemaInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.message)])),
    };
    let detail = match spec.lookup_schema(&input.schema) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!("{e}"))]));
        }
    };

    let content = Content::json(&detail).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize schema detail: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult::success(vec![content]))
}

fn call_read_resource(arguments: Option<&Map<String, Value>>) -> CallToolResult {
    let input: ReadResourceInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return CallToolResult::error(vec![Content::text(e.message)]),
    };

    let entry = onshape_mcp_resources::RESOURCES
        .iter()
        .find(|e| e.uri == input.uri);

    if let Some(entry) = entry {
        CallToolResult::success(vec![Content::text(entry.content)])
    } else {
        let available: Vec<&str> = onshape_mcp_resources::RESOURCES
            .iter()
            .map(|e| e.uri)
            .collect();
        CallToolResult::error(vec![Content::text(format!(
            "Resource not found: {}. Available URIs: {}",
            input.uri,
            available.join(", ")
        ))])
    }
}

// ============================================================================
// Error Enum Lookup
// ============================================================================

/// Embedded JSON mapping of `ErrorStringEnum` values to human-readable messages.
///
/// Generated by `scripts/generate-error-enums.py` from the `FeatureScript`
/// standard library (MIT licensed, Copyright (c) 2013-Present PTC Inc.).
const ERROR_ENUMS_JSON: &str = include_str!("../error-enums.json");

/// Lazily parsed error enum mapping.
#[allow(clippy::expect_used)]
fn error_enum_map() -> &'static HashMap<String, String> {
    use std::sync::OnceLock;

    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let parsed: Value =
            serde_json::from_str(ERROR_ENUMS_JSON).expect("embedded error-enums.json is valid");
        let enums = parsed
            .get("enums")
            .and_then(Value::as_object)
            .expect("error-enums.json has an 'enums' object");
        enums
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
            .collect()
    })
}

fn call_error_lookup(arguments: Option<&Map<String, Value>>) -> CallToolResult {
    let input: ErrorLookupInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return CallToolResult::error(vec![Content::text(e.message)]),
    };

    if input.values.is_empty() {
        return CallToolResult::error(vec![Content::text(
            "No enum values provided. Pass one or more ErrorStringEnum names \
             (e.g., [\"FILLET_FAILED\", \"SWEEP_PATH_FAILED\"]).",
        )]);
    }

    let map = error_enum_map();
    let mut output = String::new();
    for name in &input.values {
        use std::fmt::Write;
        if let Some(msg) = map.get(name.as_str()) {
            let _ = writeln!(output, "{name}: {msg}");
        } else {
            let _ = writeln!(output, "{name}: (unknown — not found in ErrorStringEnum)");
        }
    }

    CallToolResult::success(vec![Content::text(output)])
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

#[allow(clippy::too_many_lines)]
fn call_screenshot(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolEffect {
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

    for (name, value) in [
        ("did", input.did.as_str()),
        ("wvmid", input.wvmid.as_str()),
        ("eid", input.eid.as_str()),
    ] {
        if value.trim().is_empty() {
            return tool_input_error(format!("{name} must not be empty"));
        }
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

    // --- Prepare data for the continuation ---

    let label = view_label(&input.view);
    let output_path = match validate_file_path(&input.output_path) {
        Ok(p) => p,
        Err(msg) => return tool_input_error(msg),
    };

    let request = match spec.build_request(
        "getPartStudioShadedViews",
        &path_params,
        &query_params,
        &HeaderMap::new(),
        None,
    ) {
        Ok(req) => req,
        Err(e) => {
            return tool_input_error(format!("failed to build shaded views request: {e}"));
        }
    };

    // --- Return the two-phase effect: API call, then file writes ---

    ToolEffect::ApiRequest {
        request,
        continuation: Continuation::ProcessScreenshotResponse {
            output_path,
            label,
            view_matrix,
        },
    }
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
            CallToolResult::success(vec![
                Content::json(&structured)
                    .unwrap_or_else(|_| Content::text(structured.to_string())),
                Content::text(summary),
            ])
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
            CallToolResult::error(vec![
                Content::json(&structured)
                    .unwrap_or_else(|_| Content::text(structured.to_string())),
                Content::text(summary),
            ])
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

    use crate::ValidationState;

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

    fn assert_done_ok(effect: ToolEffect) -> CallToolResult {
        match effect {
            ToolEffect::Done(Ok(r)) => r,
            other => panic!("expected Done(Ok), got {other:?}"),
        }
    }

    fn assert_done_err(effect: ToolEffect) -> ErrorData {
        match effect {
            ToolEffect::Done(Err(e)) => e,
            other => panic!("expected Done(Err), got {other:?}"),
        }
    }

    /// Asserts that `effect` is a tool-level error (`is_error: Some(true)`)
    /// and returns the concatenated text content for further assertions.
    fn assert_tool_error(effect: ToolEffect) -> String {
        match effect {
            ToolEffect::Done(Ok(r)) => {
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
            ToolEffect::Done(Err(e)) => {
                panic!("expected tool error (is_error=true), got protocol error: {e:?}")
            }
            other => panic!("expected Done tool error, got {other:?}"),
        }
    }

    fn assert_api_request(effect: ToolEffect) -> (ApiRequest, Continuation) {
        match effect {
            ToolEffect::ApiRequest {
                request,
                continuation,
            } => (request, continuation),
            other => panic!("expected ApiRequest, got {other:?}"),
        }
    }

    fn assert_write_files(effect: ToolEffect) -> (Vec<FileWrite>, Continuation) {
        match effect {
            ToolEffect::WriteFiles {
                files,
                continuation,
            } => (files, continuation),
            other => panic!("expected WriteFiles, got {other:?}"),
        }
    }

    fn assert_oauth_login_flow(effect: ToolEffect) -> LoginMode {
        match effect {
            ToolEffect::OAuthLoginFlow { mode } => mode,
            other => panic!("expected OAuthLoginFlow, got {other:?}"),
        }
    }

    // --- list_tools tests ---

    #[test]
    fn list_tools_includes_auth_status() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_auth_status"));
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
        assert!(tools.iter().any(|t| t.name == "onshape_auth_login"));
    }

    #[test]
    fn list_tools_includes_api_schema() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_api_schema"));
    }

    #[test]
    fn list_tools_includes_error_lookup() {
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "onshape_error_lookup"));
    }

    #[test]
    fn list_tools_has_eleven_tools() {
        let tools = list_tools();
        assert_eq!(tools.len(), 11);
    }

    // --- auth_status tests ---

    #[test]
    fn call_tool_auth_status_returns_not_configured() {
        let auth = not_configured();
        let result = call_tool(
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
        let err = assert_done_err(call_tool(
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
        let call_result = assert_done_ok(call_tool(
            "onshape_auth_status",
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
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
        let call_result = assert_done_ok(result);
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
        let call_result = assert_done_ok(result);

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

        let err = assert_done_err(call_tool(
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
        let call_result = assert_done_ok(result);

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

        let result = assert_done_ok(call_tool(
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
        let (request, _continuation) = assert_api_request(result);
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

        let err = assert_done_err(call_tool(
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
        let (request, _continuation) = assert_api_request(result);
        assert_eq!(request.path, "/documents/search");

        let body = request
            .body
            .expect("request should have a body")
            .as_json()
            .expect("should be JSON body")
            .clone();
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
        let (request, _continuation) = assert_api_request(result);
        assert_eq!(request.method, http::Method::GET);
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
        let result = process_api_response(200, &[], body.as_bytes()).expect("should succeed");
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn process_api_response_success_plain_text() {
        let result =
            process_api_response(200, &[], b"plain text response").expect("should succeed");
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn process_api_response_success_binary() {
        let headers = [(
            "content-type".to_string(),
            "application/octet-stream".to_string(),
        )];

        let result = process_api_response(200, &headers, &[0xff]).expect("should succeed");

        assert_eq!(result.is_error, Some(false));
        let text = result.content[0]
            .raw
            .as_text()
            .expect("should be text content");
        let value: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(value["status"], 200);
        assert_eq!(value["contentType"], "application/octet-stream");
        assert_eq!(value["encoding"], "base64");
        assert_eq!(value["byteLength"], 1);
        assert_eq!(value["body"], "/w==");
    }

    #[test]
    fn process_api_response_error() {
        let result = process_api_response(404, &[], b"Not found").expect("should succeed");
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
    #[test]
    fn auth_status_validate_false_returns_immediate() {
        let auth = basic_ready();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(false));

        let result = call_tool(
            "onshape_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
        assert_eq!(call_result.is_error, Some(false));
    }

    #[test]
    fn auth_status_validate_absent_returns_immediate() {
        let auth = basic_ready();
        let result = call_tool(
            "onshape_auth_status",
            None,
            &auth,
            &default_validation(),
            None,
        );
        let call_result = assert_done_ok(result);
        assert_eq!(call_result.is_error, Some(false));
    }

    #[test]
    fn auth_status_validate_true_without_spec_returns_error() {
        let auth = basic_ready();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let err = assert_done_err(result);
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn auth_status_validate_true_returns_api_request() {
        let auth = basic_ready();
        let spec = test_spec_with_session_info();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (request, continuation) = assert_api_request(result);
        assert_eq!(request.path, "/users/sessioninfo");
        assert!(matches!(
            continuation,
            Continuation::ProcessAuthValidation { .. }
        ));
    }

    #[test]
    fn auth_status_callback_200_returns_valid_with_side_effect() {
        let auth = basic_ready();
        let spec = test_spec_with_session_info();
        let mut args = Map::new();
        args.insert("validate".to_string(), Value::Bool(true));

        let result = call_tool(
            "onshape_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, continuation) = assert_api_request(result);

        let (tool_effect, side_effects) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 200,
                headers: &[],
                body: br#"{"id": "user123"}"#,
            },
        );

        // Should return a Done result with status: valid.
        let call_result = assert_done_ok(tool_effect);
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
            "onshape_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, continuation) = assert_api_request(result);

        let (tool_effect, side_effects) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 401,
                headers: &[],
                body: b"Unauthorized",
            },
        );

        let call_result = assert_done_ok(tool_effect);
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
            "onshape_auth_status",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let (_request, continuation) = assert_api_request(result);

        let (tool_effect, side_effects) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 500,
                headers: &[],
                body: b"Internal Server Error",
            },
        );

        // Should still return a Done result, but no side effects.
        let _call_result = assert_done_ok(tool_effect);
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
        let result = call_tool("onshape_auth_status", None, &auth, &validation, None);
        let call_result = assert_done_ok(result);
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
        let call_result = assert_done_ok(result);
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
            text.text.contains("insights:sketch"),
            "should list sketch URI"
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
        let call_result = assert_done_ok(result);
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
        let result = assert_done_ok(result);
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
    // Error Lookup Tool Tests
    // ====================================================================

    #[test]
    fn call_tool_error_lookup_resolves_known_enum() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "values".to_string(),
            Value::Array(vec![Value::String("FILLET_FAILED".to_string())]),
        );
        let result = call_tool(
            "onshape_error_lookup",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let result = assert_done_ok(result);
        assert_eq!(result.is_error, Some(false));
        let text = result.content.first().expect("should have content");
        let text = match text.raw {
            rmcp::model::RawContent::Text(ref t) => &t.text,
            _ => panic!("expected text content"),
        };
        assert!(
            text.contains("FILLET_FAILED"),
            "result should contain the enum name: {text}",
        );
        // Should have a human-readable message, not "(unknown"
        assert!(
            !text.contains("(unknown"),
            "FILLET_FAILED should resolve to a message: {text}",
        );
    }

    #[test]
    fn call_tool_error_lookup_unknown_enum_says_not_found() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "values".to_string(),
            Value::Array(vec![Value::String("TOTALLY_MADE_UP_ENUM".to_string())]),
        );
        let result = call_tool(
            "onshape_error_lookup",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let result = assert_done_ok(result);
        assert_eq!(result.is_error, Some(false));
        let text = result.content.first().expect("should have content");
        let text = match text.raw {
            rmcp::model::RawContent::Text(ref t) => &t.text,
            _ => panic!("expected text content"),
        };
        assert!(
            text.contains("unknown"),
            "unknown enum should be marked as such: {text}",
        );
    }

    #[test]
    fn call_tool_error_lookup_empty_values_returns_error() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert("values".to_string(), Value::Array(vec![]));
        let result = call_tool(
            "onshape_error_lookup",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let result = assert_done_ok(result);
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn call_tool_error_lookup_multiple_values() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "values".to_string(),
            Value::Array(vec![
                Value::String("FILLET_FAILED".to_string()),
                Value::String("SWEEP_PATH_FAILED".to_string()),
                Value::String("NONEXISTENT".to_string()),
            ]),
        );
        let result = call_tool(
            "onshape_error_lookup",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        );
        let result = assert_done_ok(result);
        assert_eq!(result.is_error, Some(false));
        let text = result.content.first().expect("should have content");
        let text = match text.raw {
            rmcp::model::RawContent::Text(ref t) => &t.text,
            _ => panic!("expected text content"),
        };
        // All three should be in the output
        assert!(
            text.contains("FILLET_FAILED"),
            "should contain FILLET_FAILED"
        );
        assert!(
            text.contains("SWEEP_PATH_FAILED"),
            "should contain SWEEP_PATH_FAILED",
        );
        assert!(text.contains("NONEXISTENT"), "should contain NONEXISTENT");
        assert!(
            text.contains("unknown"),
            "NONEXISTENT should be marked unknown",
        );
    }

    // ====================================================================
    // Auth Login Tool Tests
    // ====================================================================

    #[test]
    fn auth_login_default_mode_returns_proxy() {
        let auth = not_configured();
        let result = call_tool(
            "onshape_auth_login",
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
            "onshape_auth_login",
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
            "onshape_auth_login",
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
            "onshape_auth_login",
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
            "onshape_auth_login",
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
            "onshape_auth_login",
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
    fn screenshot_output_height_zero_returns_error() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert("output_height".to_string(), Value::Number(0.into()));

        let msg = assert_tool_error(call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains("output_height"));
        assert!(msg.contains('0'));
    }

    #[test]
    fn screenshot_output_width_too_large_returns_error() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert("output_width".to_string(), Value::Number(4097.into()));

        let msg = assert_tool_error(call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains("output_width"));
        assert!(msg.contains("4097"));
    }

    #[test]
    fn screenshot_output_path_empty_returns_error() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert("output_path".to_string(), Value::String(String::new()));

        let msg = assert_tool_error(call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(
            msg.contains("file path") && msg.contains("file name"),
            "expected path validation error, got: {msg}"
        );
    }

    #[test]
    fn screenshot_output_path_traversal_returns_error() {
        let auth = not_configured();
        let spec = screenshot_spec();
        let mut args = screenshot_args(r#"{"type": "preset", "name": "front"}"#);
        args.insert(
            "output_path".to_string(),
            Value::String("../secret.png".to_string()),
        );

        let msg = assert_tool_error(call_tool(
            "onshape_screenshot",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains(".."));
    }

    #[test]
    fn screenshot_builds_api_request() {
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
        let (request, continuation) = assert_api_request(result);
        assert!(request.path.contains("/shadedviews"));
        assert!(matches!(
            continuation,
            Continuation::ProcessScreenshotResponse { .. }
        ));

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
    fn screenshot_callback_api_error_returns_done() {
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
        let (_request, continuation) = assert_api_request(result);
        let (tool_effect, side_effects) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 500,
                headers: &[],
                body: b"Internal Server Error",
            },
        );
        assert!(side_effects.is_empty());
        let call_result = assert_done_ok(tool_effect);
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
        let (_request, continuation) = assert_api_request(result);

        // Simulate a successful API response with a base64-encoded PNG.
        let fake_png = b"fake png data";
        let encoded = engine.encode(fake_png);
        let body = serde_json::json!({ "images": [encoded] }).to_string();

        let (tool_effect, side_effects) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 200,
                headers: &[],
                body: body.as_bytes(),
            },
        );
        assert!(side_effects.is_empty());

        let (files, _continuation) = assert_write_files(tool_effect);
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
        let (request, _continuation) = assert_api_request(result);

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
        let (_request, continuation) = assert_api_request(result);

        let body = serde_json::json!({ "images": [engine.encode(b"data")] }).to_string();
        let (tool_effect, _) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 200,
                headers: &[],
                body: body.as_bytes(),
            },
        );
        let (files, _continuation) = assert_write_files(tool_effect);
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

    // ====================================================================
    // Schema Lookup Tool Tests
    // ====================================================================

    fn schema_spec() -> OpenApiSpec {
        OpenApiSpec::from_json(
            r##"{
                "openapi": "3.0.1",
                "info": { "title": "Test API", "version": "1.0" },
                "servers": [{ "url": "https://cad.onshape.com/api/v1" }],
                "paths": {
                    "/features": {
                        "post": {
                            "operationId": "addFeature",
                            "summary": "Add feature",
                            "tags": ["PartStudio"],
                            "parameters": [],
                            "requestBody": {
                                "content": {
                                    "application/json;charset=UTF-8; qs=0.09": {
                                        "schema": {
                                            "$ref": "#/components/schemas/BTFeatureDefCall-1406"
                                        }
                                    }
                                }
                            },
                            "responses": { "200": {} }
                        }
                    }
                },
                "components": {
                    "schemas": {
                        "BTFeatureDefCall-1406": {
                            "type": "object",
                            "properties": {
                                "btType": { "type": "string" },
                                "feature": { "$ref": "#/components/schemas/BTMFeature-134" }
                            }
                        },
                        "BTMFeature-134": {
                            "type": "object",
                            "properties": {
                                "btType": { "type": "string" },
                                "featureId": { "type": "string" },
                                "name": { "type": "string" }
                            },
                            "discriminator": {
                                "propertyName": "btType",
                                "mapping": {
                                    "BTMSketch-151": "#/components/schemas/BTMSketch-151"
                                }
                            }
                        },
                        "BTMSketch-151": {
                            "type": "object",
                            "properties": { "btType": { "type": "string" } },
                            "allOf": [
                                { "$ref": "#/components/schemas/BTMFeature-134" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "btType": { "type": "string" },
                                        "entities": { "type": "array" }
                                    }
                                }
                            ]
                        }
                    }
                }
            }"##,
        )
        .expect("schema test spec should parse")
    }

    #[test]
    fn api_schema_returns_detail() {
        let auth = not_configured();
        let spec = schema_spec();
        let mut args = Map::new();
        args.insert(
            "schema".to_string(),
            Value::String("BTMFeature-134".to_string()),
        );

        let result = call_tool(
            "onshape_api_schema",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let call_result = assert_done_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let detail: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(detail["name"], "BTMFeature-134");
        assert_eq!(detail["discriminator_property"], "btType");

        let subtypes = detail["subtypes"].as_array().expect("should have subtypes");
        assert!(subtypes.contains(&Value::String("BTMSketch-151".to_string())));
    }

    #[test]
    fn api_schema_subtype_includes_parent_properties() {
        let auth = not_configured();
        let spec = schema_spec();
        let mut args = Map::new();
        args.insert(
            "schema".to_string(),
            Value::String("BTMSketch-151".to_string()),
        );

        let result = call_tool(
            "onshape_api_schema",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let call_result = assert_done_ok(result);
        assert_eq!(call_result.is_error, Some(false));

        let content = &call_result.content[0];
        let text = content.raw.as_text().expect("should be text content");
        let detail: Value = serde_json::from_str(&text.text).expect("should be valid JSON");
        assert_eq!(detail["parent"], "BTMFeature-134");

        let props = detail["properties"].as_object().expect("should be object");
        // Inherited from BTMFeature-134
        assert!(
            props.contains_key("featureId"),
            "should have inherited featureId"
        );
        assert!(props.contains_key("name"), "should have inherited name");
        // Own property
        assert!(props.contains_key("entities"), "should have own entities");
    }

    #[test]
    fn api_schema_nonexistent_returns_tool_error() {
        let auth = not_configured();
        let spec = schema_spec();
        let mut args = Map::new();
        args.insert(
            "schema".to_string(),
            Value::String("NonExistent-999".to_string()),
        );

        let result = call_tool(
            "onshape_api_schema",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        let call_result = assert_done_ok(result);
        assert_eq!(
            call_result.is_error,
            Some(true),
            "nonexistent schema should return tool error"
        );
    }

    #[test]
    fn api_schema_without_spec_returns_error() {
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "schema".to_string(),
            Value::String("BTMFeature-134".to_string()),
        );

        let err = assert_done_err(call_tool(
            "onshape_api_schema",
            Some(&args),
            &auth,
            &default_validation(),
            None,
        ));
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }

    // ====================================================================
    // File path validation
    // ====================================================================

    #[test]
    fn validate_file_path_accepts_absolute_path() {
        let result = validate_file_path("/tmp/data.txt");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("should be valid"),
            PathBuf::from("/tmp/data.txt")
        );
    }

    #[test]
    fn validate_file_path_accepts_relative_path() {
        let result = validate_file_path("data/file.bin");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_file_path_rejects_empty() {
        let result = validate_file_path("");
        assert!(result.is_err());
        assert!(result.expect_err("should be err").contains("file name"));
    }

    #[test]
    fn validate_file_path_rejects_whitespace_only() {
        let result = validate_file_path("   ");
        assert!(result.is_err());
    }

    #[test]
    fn validate_file_path_rejects_traversal() {
        let result = validate_file_path("/tmp/../etc/passwd");
        assert!(result.is_err());
        assert!(result.expect_err("should be err").contains(".."));
    }

    #[test]
    fn validate_file_path_rejects_relative_traversal() {
        let result = validate_file_path("../secret.txt");
        assert!(result.is_err());
    }

    // ====================================================================
    // File reference types
    // ====================================================================

    #[test]
    fn file_encoding_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FileEncoding::TextUtf8).expect("should serialize"),
            r#""text_utf8""#
        );
        assert_eq!(
            serde_json::to_string(&FileEncoding::Base64).expect("should serialize"),
            r#""base64""#
        );
        assert_eq!(
            serde_json::to_string(&FileEncoding::RawBytes).expect("should serialize"),
            r#""raw_bytes""#
        );
    }

    #[test]
    fn file_encoding_deserializes_snake_case() {
        let enc: FileEncoding = serde_json::from_str(r#""text_utf8""#).expect("should deserialize");
        assert!(matches!(enc, FileEncoding::TextUtf8));
    }

    #[test]
    fn file_reference_roundtrips() {
        let fr = FileReference {
            path: "/tmp/test.fs".to_string(),
            field: "file".to_string(),
            encoding: FileEncoding::RawBytes,
        };
        let json = serde_json::to_string(&fr).expect("should serialize");
        let fr2: FileReference = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(fr2.path, fr.path);
        assert_eq!(fr2.field, fr.field);
    }

    #[test]
    fn api_call_input_file_refs_default_empty() {
        let input: ApiCallInput =
            serde_json::from_str(r#"{"endpoint": "getDocuments"}"#).expect("should parse");
        assert!(input.file_refs.is_empty());
        assert!(input.header_params.is_empty());
    }

    #[test]
    fn api_call_input_with_header_params_parses() {
        let input: ApiCallInput = serde_json::from_str(
            r#"{
                "endpoint": "getDocuments",
                "header_params": {"Accept": "application/octet-stream"}
            }"#,
        )
        .expect("should parse");

        assert_eq!(input.header_params["Accept"], "application/octet-stream");
    }

    #[test]
    fn api_call_input_with_file_refs_parses() {
        let input: ApiCallInput = serde_json::from_str(
            r#"{
                "endpoint": "uploadFileCreateElement",
                "file_refs": [
                    {"path": "/tmp/part.x_t", "field": "file", "encoding": "raw_bytes"}
                ]
            }"#,
        )
        .expect("should parse");
        assert_eq!(input.file_refs.len(), 1);
        assert_eq!(input.file_refs[0].path, "/tmp/part.x_t");
        assert_eq!(input.file_refs[0].field, "file");
        assert!(matches!(
            input.file_refs[0].encoding,
            FileEncoding::RawBytes
        ));
    }

    // ====================================================================
    // call_api_call with file_refs
    // ====================================================================

    fn assert_read_files(effect: ToolEffect) -> (Vec<FileRead>, Continuation) {
        match effect {
            ToolEffect::ReadFiles {
                reads,
                continuation,
            } => (reads, continuation),
            other => panic!("expected ReadFiles, got {other:?}"),
        }
    }

    #[test]
    fn api_call_without_file_refs_returns_api_request() {
        let spec = test_spec();
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocuments".to_string()),
        );

        let effect = call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );
        // Should be a direct ApiRequest, no ReadFiles indirection.
        let (_request, continuation) = assert_api_request(effect);
        assert!(matches!(continuation, Continuation::FormatApiResponse));
    }

    #[test]
    fn api_call_header_params_reach_request() {
        let spec = test_spec();
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocuments".to_string()),
        );
        args.insert(
            "header_params".to_string(),
            serde_json::json!({"Accept": "application/octet-stream"}),
        );

        let effect = call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );

        let (request, continuation) = assert_api_request(effect);
        assert!(matches!(continuation, Continuation::FormatApiResponse));
        assert_eq!(request.headers["Accept"], "application/octet-stream");
    }

    #[test]
    fn api_call_invalid_header_name_returns_error() {
        let spec = test_spec();
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("getDocuments".to_string()),
        );
        args.insert(
            "header_params".to_string(),
            serde_json::json!({"not a header": "value"}),
        );

        let msg = assert_tool_error(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(
            msg.contains("invalid header name"),
            "expected invalid header error, got: {msg}"
        );
    }

    #[test]
    fn api_call_with_file_refs_returns_read_files() {
        let spec = test_spec();
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert(
            "body".to_string(),
            Value::String(r#"{"limit": 10}"#.to_string()),
        );
        args.insert(
            "file_refs".to_string(),
            serde_json::from_str(
                r#"[{"path": "/tmp/query.txt", "field": "rawQuery", "encoding": "text_utf8"}]"#,
            )
            .expect("should parse file_refs JSON"),
        );

        let effect = call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        );

        let (reads, continuation) = assert_read_files(effect);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].path, PathBuf::from("/tmp/query.txt"));
        assert!(matches!(
            continuation,
            Continuation::InjectFilesIntoRequest { .. }
        ));
    }

    #[test]
    fn api_call_file_ref_invalid_path_returns_error() {
        let spec = test_spec();
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert(
            "file_refs".to_string(),
            serde_json::from_str(
                r#"[{"path": "../etc/passwd", "field": "rawQuery", "encoding": "text_utf8"}]"#,
            )
            .expect("should parse file_refs JSON"),
        );

        let msg = assert_tool_error(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(msg.contains(".."), "expected traversal error, got: {msg}");
    }

    #[test]
    fn api_call_file_ref_empty_path_returns_error() {
        let spec = test_spec();
        let auth = not_configured();
        let mut args = Map::new();
        args.insert(
            "endpoint".to_string(),
            Value::String("searchDocuments".to_string()),
        );
        args.insert(
            "file_refs".to_string(),
            serde_json::from_str(r#"[{"path": "", "field": "rawQuery", "encoding": "text_utf8"}]"#)
                .expect("should parse file_refs JSON"),
        );

        let msg = assert_tool_error(call_tool(
            "onshape_api_call",
            Some(&args),
            &auth,
            &default_validation(),
            Some(&spec),
        ));
        assert!(
            msg.contains("file path"),
            "expected path validation error, got: {msg}"
        );
    }

    // ====================================================================
    // resume() with InjectFilesIntoRequest — JSON body
    // ====================================================================

    /// Build a minimal `ApiRequest` with a JSON body for injection tests.
    fn json_request_for_injection(body: Value) -> ApiRequest {
        use onshape_client_core::request::RequestBody;
        ApiRequest {
            method: http::Method::POST,
            path: "/test".to_string(),
            query_params: vec![],
            headers: http::HeaderMap::default(),
            body: Some(RequestBody::Json(body)),
            content_type: Some("application/json".to_string()),
        }
    }

    #[test]
    fn resume_inject_text_utf8_into_json_body() {
        let request = json_request_for_injection(serde_json::json!({"existing": "value"}));
        let file_refs = vec![FileReference {
            path: "/tmp/script.fs".to_string(),
            field: "content".to_string(),
            encoding: FileEncoding::TextUtf8,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/script.fs"),
            data: b"function hello() {}".to_vec(),
        }];

        let (effect, side_effects) = resume(continuation, IoResult::FileReadResults(&results));
        assert!(side_effects.is_empty());

        let (req, cont) = assert_api_request(effect);
        assert!(matches!(cont, Continuation::FormatApiResponse));

        let body = req.body.expect("should have body");
        let json = body.as_json().expect("should be JSON");
        assert_eq!(json["existing"], "value");
        assert_eq!(json["content"], "function hello() {}");
    }

    #[test]
    fn resume_inject_base64_into_json_body() {
        let request = json_request_for_injection(serde_json::json!({}));
        let file_refs = vec![FileReference {
            path: "/tmp/data.bin".to_string(),
            field: "payload".to_string(),
            encoding: FileEncoding::Base64,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let raw_data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/data.bin"),
            data: raw_data.clone(),
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let (req, _) = assert_api_request(effect);

        let json = req.body.expect("should have body");
        let encoded = json.as_json().expect("should be JSON")["payload"]
            .as_str()
            .expect("should be string");
        let engine = base64::engine::general_purpose::STANDARD;
        let decoded = base64::Engine::decode(&engine, encoded).expect("should decode base64");
        assert_eq!(decoded, raw_data);
    }

    #[test]
    fn resume_inject_raw_bytes_into_json_body_returns_error() {
        let request = json_request_for_injection(serde_json::json!({}));
        let file_refs = vec![FileReference {
            path: "/tmp/data.bin".to_string(),
            field: "payload".to_string(),
            encoding: FileEncoding::RawBytes,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/data.bin"),
            data: vec![0xFF],
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let msg = assert_tool_error(effect);
        assert!(
            msg.contains("raw_bytes") && msg.contains("JSON"),
            "expected raw_bytes + JSON error, got: {msg}"
        );
    }

    #[test]
    fn resume_inject_invalid_utf8_returns_error() {
        let request = json_request_for_injection(serde_json::json!({}));
        let file_refs = vec![FileReference {
            path: "/tmp/bad.txt".to_string(),
            field: "content".to_string(),
            encoding: FileEncoding::TextUtf8,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/bad.txt"),
            data: vec![0xFF, 0xFE], // invalid UTF-8
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let msg = assert_tool_error(effect);
        assert!(msg.contains("UTF-8"), "expected UTF-8 error, got: {msg}");
    }

    // ====================================================================
    // resume() with InjectFilesIntoRequest — multipart body
    // ====================================================================

    /// Build a minimal `ApiRequest` with a multipart body for injection tests.
    fn multipart_request_for_injection(text_fields: Vec<(String, String)>) -> ApiRequest {
        use onshape_client_core::request::{MultipartBody, RequestBody};
        ApiRequest {
            method: http::Method::POST,
            path: "/upload".to_string(),
            query_params: vec![],
            headers: http::HeaderMap::default(),
            body: Some(RequestBody::Multipart(MultipartBody {
                text_fields,
                binary_fields: vec![],
            })),
            content_type: Some("multipart/form-data".to_string()),
        }
    }

    #[test]
    fn resume_inject_raw_bytes_into_multipart_body() {
        let request = multipart_request_for_injection(vec![(
            "formatName".to_string(),
            "PARASOLID".to_string(),
        )]);
        let file_refs = vec![FileReference {
            path: "/tmp/part.x_t".to_string(),
            field: "file".to_string(),
            encoding: FileEncoding::RawBytes,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let file_data = b"binary-file-content".to_vec();
        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/part.x_t"),
            data: file_data.clone(),
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let (req, _) = assert_api_request(effect);

        match req.body.expect("should have body") {
            RequestBody::Multipart(m) => {
                assert_eq!(m.binary_fields.len(), 1);
                assert_eq!(m.binary_fields[0].field_name, "file");
                assert_eq!(m.binary_fields[0].data, file_data);
                // Existing text fields preserved.
                assert!(
                    m.text_fields
                        .iter()
                        .any(|(k, v)| k == "formatName" && v == "PARASOLID")
                );
            }
            RequestBody::Json(j) => panic!("expected Multipart, got Json({j:?})"),
        }
    }

    #[test]
    fn resume_inject_text_utf8_into_multipart_body() {
        let request = multipart_request_for_injection(vec![]);
        let file_refs = vec![FileReference {
            path: "/tmp/script.fs".to_string(),
            field: "contents".to_string(),
            encoding: FileEncoding::TextUtf8,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/script.fs"),
            data: b"FeatureScript 2244;".to_vec(),
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let (req, _) = assert_api_request(effect);

        match req.body.expect("should have body") {
            RequestBody::Multipart(m) => {
                assert!(
                    m.text_fields
                        .iter()
                        .any(|(k, v)| k == "contents" && v == "FeatureScript 2244;"),
                    "text field not found: {:?}",
                    m.text_fields
                );
            }
            RequestBody::Json(j) => panic!("expected Multipart, got Json({j:?})"),
        }
    }

    #[test]
    fn resume_inject_base64_into_multipart_body() {
        let request = multipart_request_for_injection(vec![]);
        let file_refs = vec![FileReference {
            path: "/tmp/data.bin".to_string(),
            field: "encoded_data".to_string(),
            encoding: FileEncoding::Base64,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let raw_data = vec![0x01, 0x02, 0x03];
        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/data.bin"),
            data: raw_data.clone(),
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let (req, _) = assert_api_request(effect);

        match req.body.expect("should have body") {
            RequestBody::Multipart(m) => {
                let engine = base64::engine::general_purpose::STANDARD;
                let expected = base64::Engine::encode(&engine, &raw_data);
                assert!(
                    m.text_fields
                        .iter()
                        .any(|(k, v)| k == "encoded_data" && v == &expected)
                );
            }
            RequestBody::Json(j) => panic!("expected Multipart, got Json({j:?})"),
        }
    }

    // ====================================================================
    // resume() with InjectFilesIntoRequest — error cases
    // ====================================================================

    #[test]
    fn resume_inject_file_read_error_returns_tool_error() {
        let request = json_request_for_injection(serde_json::json!({}));
        let file_refs = vec![FileReference {
            path: "/tmp/missing.txt".to_string(),
            field: "content".to_string(),
            encoding: FileEncoding::TextUtf8,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![FileReadResult::Error {
            path: PathBuf::from("/tmp/missing.txt"),
            message: "No such file or directory".to_string(),
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let msg = assert_tool_error(effect);
        assert!(
            msg.contains("missing.txt") && msg.contains("No such file"),
            "expected read error, got: {msg}"
        );
    }

    #[test]
    fn resume_inject_no_body_returns_error() {
        let request = ApiRequest {
            method: http::Method::GET,
            path: "/test".to_string(),
            query_params: vec![],
            headers: http::HeaderMap::default(),
            body: None,
            content_type: None,
        };
        let file_refs = vec![FileReference {
            path: "/tmp/data.txt".to_string(),
            field: "content".to_string(),
            encoding: FileEncoding::TextUtf8,
        }];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![FileReadResult::Success {
            path: PathBuf::from("/tmp/data.txt"),
            data: b"hello".to_vec(),
        }];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let msg = assert_tool_error(effect);
        assert!(
            msg.contains("no request body"),
            "expected no-body error, got: {msg}"
        );
    }

    #[test]
    fn resume_inject_multiple_files_into_json_body() {
        let request = json_request_for_injection(serde_json::json!({"base": true}));
        let file_refs = vec![
            FileReference {
                path: "/tmp/a.txt".to_string(),
                field: "field_a".to_string(),
                encoding: FileEncoding::TextUtf8,
            },
            FileReference {
                path: "/tmp/b.bin".to_string(),
                field: "field_b".to_string(),
                encoding: FileEncoding::Base64,
            },
        ];
        let continuation = Continuation::InjectFilesIntoRequest { request, file_refs };

        let results = vec![
            FileReadResult::Success {
                path: PathBuf::from("/tmp/a.txt"),
                data: b"alpha".to_vec(),
            },
            FileReadResult::Success {
                path: PathBuf::from("/tmp/b.bin"),
                data: vec![0xCA, 0xFE],
            },
        ];

        let (effect, _) = resume(continuation, IoResult::FileReadResults(&results));
        let (req, _) = assert_api_request(effect);
        let json = req
            .body
            .expect("should have body")
            .as_json()
            .expect("should be JSON")
            .clone();

        assert_eq!(json["base"], true);
        assert_eq!(json["field_a"], "alpha");
        // field_b should be base64-encoded
        let engine = base64::engine::general_purpose::STANDARD;
        let decoded =
            base64::Engine::decode(&engine, json["field_b"].as_str().expect("should be string"))
                .expect("should decode base64");
        assert_eq!(decoded, vec![0xCA, 0xFE]);
    }
}
