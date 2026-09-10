//! Pure `OpenAPI` tool handlers.
//!
//! The host supplies endpoint-specific body validation. Tool names, descriptions,
//! public input types, and shared I/O effects are owned by the parent module.

use std::collections::HashMap;
use std::path::PathBuf;

use http::{HeaderMap, HeaderName, HeaderValue};
use onshape_client_core::request::RequestBody;
use onshape_openapi::{OpenApiSpec, SearchFilters};
use rmcp::{
    ErrorData,
    model::{CallToolResult, ContentBlock, ErrorCode},
};
use serde_json::{Map, Value};

use super::{
    ApiCallInput, ApiExplainInput, ApiSchemaInput, ApiSearchInput, Continuation, FileEncoding,
    FileRead, FileReference, ToolEffect, parse_arguments, tool_input_error, validate_file_path,
};

/// Host validation of the decoded body before request construction and file reads.
pub(super) type BodyValidator = fn(&str, Option<&Value>, &[FileReference]) -> Result<(), String>;

pub(super) fn search(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiSearchInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![ContentBlock::text(e.message)])),
    };
    let filters = SearchFilters {
        method: input.method,
        tag: input.tag,
    };
    let results = spec.search(&input.query, &filters);

    let content = ContentBlock::json(&results).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize search results: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult::success(vec![content]))
}

pub(super) fn explain(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiExplainInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![ContentBlock::text(e.message)])),
    };
    let detail = match spec.explain(&input.endpoint) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{e}"
            ))]));
        }
    };

    let content = ContentBlock::json(&detail).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize endpoint detail: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult::success(vec![content]))
}

/// Decode the body, apply host validation, then prepare the request and file reads.
/// The validator runs synchronously; continuations remain plain data.
pub(super) fn call(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
    validate_body: BodyValidator,
) -> ToolEffect {
    if arguments
        .and_then(|arguments| arguments.get("body"))
        .is_some_and(Value::is_null)
    {
        return tool_input_error("body must not be JSON null; omit it instead");
    }

    let input: ApiCallInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };

    let body = match input.body {
        Some(Value::String(serialized)) => match serde_json::from_str(&serialized) {
            Ok(body) => Some(body),
            Err(e) => return tool_input_error(format!("invalid body JSON: {e}")),
        },
        body => body,
    };

    if body == Some(Value::Null) {
        return tool_input_error(
            "body parsed as JSON null; omit the body field instead of passing \"null\"",
        );
    }

    if let Err(message) = validate_body(&input.endpoint, body.as_ref(), &input.file_refs) {
        return tool_input_error(message);
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

pub(super) fn schema(
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiSchemaInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![ContentBlock::text(e.message)])),
    };
    let detail = match spec.lookup_schema(&input.schema) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{e}"
            ))]));
        }
    };

    let content = ContentBlock::json(&detail).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("failed to serialize schema detail: {e}"),
            None,
        )
    })?;

    Ok(CallToolResult::success(vec![content]))
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document_store_spec() -> OpenApiSpec {
        OpenApiSpec::from_json(
            r#"{
                "openapi": "3.0.1",
                "info": { "title": "Document Store", "version": "1.0" },
                "servers": [{ "url": "https://documents.example.com" }],
                "paths": {
                    "/documents": {
                        "post": {
                            "operationId": "createDocument",
                            "requestBody": {
                                "required": true,
                                "content": {
                                    "application/json": {
                                        "schema": {
                                            "type": "object",
                                            "properties": { "title": { "type": "string" } }
                                        }
                                    }
                                }
                            },
                            "responses": { "201": { "description": "Created" } }
                        }
                    }
                }
            }"#,
        )
        .expect("document store spec should parse")
    }

    #[test]
    fn call_does_not_impose_onshape_policy_on_matching_operation_ids() {
        // This API shares Onshape's operation ID and path, but uses a title field.
        let spec = document_store_spec();
        let body = json!({ "title": "Example" });
        let arguments = json!({ "endpoint": "createDocument", "body": body });

        let ToolEffect::ApiRequest {
            request,
            continuation: Continuation::FormatApiResponse,
        } = call(arguments.as_object(), &spec, |_, _, _| Ok(()))
        else {
            panic!("a permissive host should allow the document store request");
        };

        assert_eq!(request.method, http::Method::POST);
        assert_eq!(request.path, "/documents");
        assert_eq!(
            request.body.as_ref().and_then(RequestBody::as_json),
            Some(&body)
        );
    }

    #[test]
    fn host_validation_receives_decoded_body_before_file_validation() {
        let spec = document_store_spec();
        let arguments = json!({
            "endpoint": "createDocument",
            "body": r#"{"title":"Example"}"#,
            "file_refs": [{
                "path": "../content.txt",
                "field": "content",
                "encoding": "text_utf8"
            }]
        });

        let effect = call(arguments.as_object(), &spec, |endpoint, body, file_refs| {
            assert_eq!(endpoint, "createDocument");
            assert_eq!(body, Some(&json!({ "title": "Example" })));
            assert_eq!(file_refs.len(), 1);
            assert_eq!(file_refs[0].path, "../content.txt");
            assert_eq!(file_refs[0].field, "content");
            assert!(matches!(file_refs[0].encoding, FileEncoding::TextUtf8));
            Err("host rejected document".into())
        });

        let ToolEffect::Done(Ok(result)) = effect else {
            panic!("host rejection should return a tool error before scheduling I/O");
        };
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.content.len(), 1);
        assert_eq!(
            result.content[0].as_text().expect("text diagnostic").text,
            "host rejected document"
        );
    }
}
