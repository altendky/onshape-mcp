//! Generic API effects, file injection, and response formatting.
//!
//! The host supplies synchronous presentation, validation, and diagnostic policy.
//! Effects and continuations contain only data, with no authentication or I/O.

use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine;
use onshape_openapi::request::{ApiRequest, BinaryField, MultipartBody, RequestBody};
use onshape_openapi::{EndpointDetail, OpenApiSpec, SchemaDetail};
use rmcp::{
    ErrorData,
    model::{CallToolResult, ContentBlock, ErrorCode},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Host validation of the decoded body before request construction and file reads.
pub type BodyValidator = fn(&str, Option<&Value>, &[FileReference]) -> Result<(), String>;

/// Synchronous host policy, supplied separately from plain-data continuations.
pub struct Policy {
    /// Customize endpoint details before serialization without changing the spec.
    pub present_endpoint: fn(&mut EndpointDetail, &OpenApiSpec),
    /// Customize component schema details before serialization without changing the spec.
    pub present_schema: fn(&mut SchemaDetail, &OpenApiSpec),
    /// Validate the decoded body before generic file and request validation.
    pub validate_body: BodyValidator,
    /// Validate the final request after file contents have been injected.
    pub validate_request: fn(&ApiRequest) -> Result<(), String>,
    /// Append host-approved details to a generic HTTP error diagnostic.
    pub append_error_details: fn(&mut String, &[u8]),
}

/// A generic tool result or I/O operation to execute.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Effect {
    /// The tool completed.
    Done(Result<CallToolResult, ErrorData>),
    /// Execute a neutral API request, then resume with its response.
    ApiRequest {
        /// The request to execute.
        request: ApiRequest,
        /// How to process the response.
        continuation: Continuation,
    },
    /// Read local files, then resume with their contents.
    ReadFiles {
        /// Files to read.
        reads: Vec<FileRead>,
        /// How to process the read results.
        continuation: Continuation,
    },
}

/// Plain data describing the next step of generic API tool execution.
#[allow(clippy::large_enum_variant)] // Keep prepared requests inline across host adaptation.
#[derive(Debug)]
pub enum Continuation {
    /// Format an HTTP response as the final tool result.
    FormatApiResponse,
    /// Inject file contents into a prepared request before HTTP execution.
    InjectFilesIntoRequest {
        /// The request awaiting file contents.
        request: ApiRequest,
        /// Fields and encodings to use for injection.
        file_refs: Vec<FileReference>,
    },
}

/// The result of a generic I/O effect, supplied by the host.
pub enum IoResult<'a> {
    /// An HTTP response.
    ApiResponse {
        /// HTTP status code.
        status: u16,
        /// Response headers as name/value pairs.
        headers: &'a [(String, String)],
        /// Raw response bytes.
        body: &'a [u8],
    },
    /// Results of file read operations.
    FileReadResults(&'a [FileReadResult]),
}

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

/// Create an [`Effect`] for an expected user-input error.
///
/// Returns a successful `CallToolResult` with `is_error: Some(true)`, keeping
/// the MCP transport clean. Use this for validation failures that the caller
/// (typically an LLM) can act on — as opposed to protocol-level
/// `Err(ErrorData)` which signals handler/infrastructure breakage.
pub fn tool_input_error(message: impl Into<String>) -> Effect {
    Effect::Done(Ok(CallToolResult::error(vec![ContentBlock::text(
        message.into(),
    )])))
}

/// Validate a file path for use in file I/O effects.
///
/// Returns `Ok(PathBuf)` if the path is valid. Returns `Err(message)` if:
/// - The path is empty or whitespace-only
/// - The path has no file name component
/// - The path contains `..` segments (directory traversal)
pub fn validate_file_path(path: &str) -> Result<PathBuf, String> {
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

const fn http_error_category(status: u16) -> &'static str {
    match status {
        400 | 422 => "invalid_request",
        401 => "authentication",
        403 => "permission",
        404 => "not_found",
        408 => "timeout",
        409 => "conflict",
        429 => "rate_limited",
        502..=504 => "service_unavailable",
        400..=499 => "client_error",
        500..=599 => "server_error",
        _ => "http_error",
    }
}

const fn http_error_is_transient(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502..=504)
}

fn retry_after_seconds(headers: &[(String, String)]) -> Option<u64> {
    const MAX_RETRY_AFTER_SECONDS: u64 = 86_400;

    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .map(|(_, value)| value.trim_matches([' ', '\t']))
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds <= MAX_RETRY_AFTER_SECONDS)
}

/// Combine generic HTTP classification with host-approved response details.
fn sanitized_api_error(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    policy: &Policy,
) -> String {
    use std::fmt::Write;

    let transient = http_error_is_transient(status);
    let mut detail = format!(
        "API error (HTTP {status}): category={}; transient={transient}",
        http_error_category(status)
    );

    (policy.append_error_details)(&mut detail, body);
    if transient && let Some(seconds) = retry_after_seconds(headers) {
        let _ = write!(detail, "; retry_after_seconds={seconds}");
    }

    detail
}

/// Convert a raw HTTP response into a [`CallToolResult`].
///
/// # Arguments
///
/// * `status` - HTTP status code
/// * `headers` - Response headers as `(name, value)` pairs
/// * `body` - Raw response body bytes
/// * `policy` - Host policy for interpreting error responses
///
/// # Errors
///
/// Returns an error if the response cannot be processed.
pub fn process_api_response(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    policy: &Policy,
) -> Result<CallToolResult, ErrorData> {
    let is_success = (200..300).contains(&status);
    if !is_success {
        return Ok(CallToolResult::error(vec![ContentBlock::text(
            sanitized_api_error(status, headers, body, policy),
        )]));
    }

    let body_text = match std::str::from_utf8(body) {
        Ok(text) if response_content_type(headers).is_none_or(content_type_is_textual) => text,
        _ => {
            let content = binary_api_response_content(status, headers, body)?;
            return Ok(CallToolResult::success(vec![content]));
        }
    };

    // Try to parse as JSON for nice formatting
    let content = if let Ok(json_val) = serde_json::from_str::<Value>(body_text) {
        // LCOV_EXCL_START — serde_json::Value is always serializable as JSON content.
        ContentBlock::json(&json_val).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("failed to serialize API response: {e}"),
                None,
            )
        })?
        // LCOV_EXCL_STOP
    } else {
        ContentBlock::text(body_text)
    };

    Ok(CallToolResult::success(vec![content]))
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
) -> Result<ContentBlock, ErrorData> {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut response = serde_json::json!({
        "status": status,
        "byteLength": body.len(),
    });

    if let Some(response) = response.as_object_mut() {
        response.insert("encoding".to_string(), Value::String("base64".to_string()));
        response.insert("body".to_string(), Value::String(engine.encode(body)));
    }

    if let Some(content_type) = response_content_type(headers)
        && let Some(response) = response.as_object_mut()
    {
        response.insert(
            "contentType".to_string(),
            Value::String(content_type.to_string()),
        );
    }

    ContentBlock::json(&response).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize binary API response metadata: {e}"),
            None,
        )
    })
}

/// Resume generic API execution after the host completes an I/O effect.
///
/// # Panics
///
/// Panics when the result does not match the continuation, indicating an I/O
/// interpreter programming error.
#[must_use]
pub fn resume(continuation: Continuation, result: IoResult<'_>, policy: &Policy) -> Effect {
    match (continuation, result) {
        (
            Continuation::FormatApiResponse,
            IoResult::ApiResponse {
                status,
                headers,
                body,
            },
        ) => Effect::Done(process_api_response(status, headers, body, policy)),
        (
            Continuation::InjectFilesIntoRequest { request, file_refs },
            IoResult::FileReadResults(results),
        ) => resume_inject_files(request, &file_refs, results, policy),
        (continuation, result) => unreachable!(
            "mismatched Continuation and IoResult: continuation={continuation:?}, result kind={}",
            match result {
                IoResult::ApiResponse { .. } => "ApiResponse",
                IoResult::FileReadResults(_) => "FileReadResults",
            }
        ),
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
    policy: &Policy,
) -> Effect {
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

    if let Err(message) = (policy.validate_request)(&request) {
        return tool_input_error(message);
    }

    Effect::ApiRequest {
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
    multipart: &mut MultipartBody,
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
