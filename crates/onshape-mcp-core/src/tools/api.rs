//! Pure `OpenAPI` tool handlers.
//!
//! The `execution` module owns generic effects, file injection, and response
//! formatting. The host supplies validation and diagnostic policy. Tool metadata
//! and the four tool input schemas are owned by the parent module.

mod execution;

use execution::tool_input_error;
pub(super) use execution::{
    Continuation, Effect, IoResult, Policy, process_api_response, resume, validate_file_path,
};
pub use execution::{FileEncoding, FileRead, FileReadResult, FileReference};

use std::collections::HashMap;
use std::path::PathBuf;

use http::{HeaderMap, HeaderName, HeaderValue};
use onshape_openapi::request::RequestBody;
use onshape_openapi::{OpenApiSpec, SearchFilters};
use rmcp::{
    ErrorData,
    model::{CallToolResult, ContentBlock, ErrorCode},
};
use serde_json::{Map, Value};

use super::{ApiCallInput, ApiExplainInput, ApiSchemaInput, ApiSearchInput};

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
    policy: &Policy,
) -> Effect {
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

    if let Err(message) = (policy.validate_body)(&input.endpoint, body.as_ref(), &input.file_refs) {
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
        Effect::ApiRequest {
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

        Effect::ReadFiles {
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

/// Parse tool arguments from the MCP request into a typed struct.
pub(super) fn parse_arguments<T: serde::de::DeserializeOwned>(
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::execution::BodyValidator;
    use super::*;
    use serde_json::json;

    fn policy(validate_body: BodyValidator) -> Policy {
        Policy {
            validate_body,
            validate_request: |_| Ok(()),
            append_error_details: |_, _| {},
        }
    }

    fn file_upload_arguments() -> Value {
        json!({
            "endpoint": "createDocument",
            "body": { "title": "Example" },
            "file_refs": [{
                "path": "content.txt",
                "field": "content",
                "encoding": "text_utf8"
            }]
        })
    }

    #[test]
    fn generic_file_read_request_response_flow() {
        let spec = document_store_spec();
        let arguments = file_upload_arguments();
        let host_policy = policy(|_, _, _| Ok(()));
        let Effect::ReadFiles {
            reads,
            continuation,
        } = call(arguments.as_object(), &spec, &host_policy)
        else {
            panic!("file references should schedule file reads");
        };
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].path, PathBuf::from("content.txt"));

        let results = [FileReadResult::Success {
            path: reads[0].path.clone(),
            data: b"Uploaded content".to_vec(),
        }];
        let Effect::ApiRequest {
            request,
            continuation,
        } = resume(
            continuation,
            IoResult::FileReadResults(&results),
            &host_policy,
        )
        else {
            panic!("file injection should produce an API request");
        };
        assert_eq!(request.method, http::Method::POST);
        assert_eq!(request.path, "/documents");
        assert_eq!(
            request.body.as_ref().and_then(RequestBody::as_json),
            Some(&json!({ "title": "Example", "content": "Uploaded content" }))
        );

        let response = br#"{"id":"item-1"}"#;
        let headers = [("content-type".into(), "application/json".into())];
        let Effect::Done(Ok(result)) = resume(
            continuation,
            IoResult::ApiResponse {
                status: 201,
                headers: &headers,
                body: response,
            },
            &host_policy,
        ) else {
            panic!("the API response should complete the tool call");
        };
        assert_ne!(result.is_error, Some(true));
        assert_eq!(result.content.len(), 1);
        let text = &result.content[0].as_text().expect("JSON text content").text;
        assert_eq!(
            serde_json::from_str::<Value>(text).expect("valid JSON"),
            json!({ "id": "item-1" })
        );
    }

    #[test]
    fn host_validation_rejects_injected_request_before_http_execution() {
        let spec = document_store_spec();
        let arguments = file_upload_arguments();
        let mut host_policy = policy(|_, _, _| Ok(()));
        host_policy.validate_request = |request| {
            assert_eq!(request.method, http::Method::POST);
            assert_eq!(request.path, "/documents");
            assert_eq!(
                request.body.as_ref().and_then(RequestBody::as_json),
                Some(&json!({ "title": "Example", "content": "Rejected content" }))
            );
            Err("host rejected injected request".into())
        };
        let Effect::ReadFiles { continuation, .. } =
            call(arguments.as_object(), &spec, &host_policy)
        else {
            panic!("file references should schedule file reads");
        };
        let results = [FileReadResult::Success {
            path: PathBuf::from("content.txt"),
            data: b"Rejected content".to_vec(),
        }];
        let Effect::Done(Ok(result)) = resume(
            continuation,
            IoResult::FileReadResults(&results),
            &host_policy,
        ) else {
            panic!("host rejection must prevent HTTP execution");
        };
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.content[0].as_text().expect("text diagnostic").text,
            "host rejected injected request"
        );
    }

    #[test]
    fn error_response_uses_host_details_and_generic_retry_guidance() {
        let mut enriched_policy = policy(|_, _, _| Ok(()));
        enriched_policy.append_error_details = |detail, body| {
            assert_eq!(body, b"private response payload");
            detail.push_str("; host_code=BUSY");
        };
        let headers = [("retry-after".into(), "30".into())];
        for (host_policy, extra) in [
            (policy(|_, _, _| Ok(())), ""),
            (enriched_policy, "; host_code=BUSY"),
        ] {
            let Effect::Done(Ok(result)) = resume(
                Continuation::FormatApiResponse,
                IoResult::ApiResponse {
                    status: 429,
                    headers: &headers,
                    body: b"private response payload",
                },
                &host_policy,
            ) else {
                panic!("an HTTP error should complete with a tool error");
            };
            assert_eq!(result.is_error, Some(true));
            assert_eq!(
                result.content[0].as_text().expect("text diagnostic").text,
                format!(
                    "API error (HTTP 429): category=rate_limited; transient=true{extra}; retry_after_seconds=30"
                )
            );
        }
    }

    #[test]
    #[should_panic(expected = "mismatched Continuation and IoResult")]
    fn response_continuation_rejects_file_results() {
        let _ = resume(
            Continuation::FormatApiResponse,
            IoResult::FileReadResults(&[]),
            &policy(|_, _, _| Ok(())),
        );
    }

    #[test]
    #[should_panic(expected = "mismatched Continuation and IoResult")]
    fn file_continuation_rejects_http_response() {
        let spec = document_store_spec();
        let arguments = file_upload_arguments();
        let host_policy = policy(|_, _, _| Ok(()));
        let Effect::ReadFiles { continuation, .. } =
            call(arguments.as_object(), &spec, &host_policy)
        else {
            panic!("file references should schedule file reads");
        };
        let _ = resume(
            continuation,
            IoResult::ApiResponse {
                status: 200,
                headers: &[],
                body: b"",
            },
            &host_policy,
        );
    }

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

        let Effect::ApiRequest {
            request,
            continuation: Continuation::FormatApiResponse,
        } = call(arguments.as_object(), &spec, &policy(|_, _, _| Ok(())))
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
    fn call_rejects_traversal_file_refs_with_permissive_host() {
        let spec = document_store_spec();
        let arguments = json!({
            "endpoint": "createDocument",
            "body": { "title": "Example" },
            "file_refs": [{
                "path": "../content.txt",
                "field": "content",
                "encoding": "text_utf8"
            }]
        });

        let Effect::Done(Ok(result)) =
            call(arguments.as_object(), &spec, &policy(|_, _, _| Ok(())))
        else {
            panic!("a traversal path must not schedule file reads");
        };
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.content.len(), 1);
        let message = &result.content[0].as_text().expect("text diagnostic").text;
        assert!(message.contains("invalid file_ref path"));
        assert!(message.contains("must not contain '..' segments"));
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

        let host_policy = policy(|endpoint, body, file_refs| {
            assert_eq!(endpoint, "createDocument");
            assert_eq!(body, Some(&json!({ "title": "Example" })));
            assert_eq!(file_refs.len(), 1);
            assert_eq!(file_refs[0].path, "../content.txt");
            assert_eq!(file_refs[0].field, "content");
            assert!(matches!(file_refs[0].encoding, FileEncoding::TextUtf8));
            Err("host rejected document".into())
        });
        let effect = call(arguments.as_object(), &spec, &host_policy);

        let Effect::Done(Ok(result)) = effect else {
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
