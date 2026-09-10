//! Pure `OpenAPI` tool handlers.
//!
//! The `execution` module owns generic effects, file injection, and response
//! formatting. Generic input types and host-configured tool metadata live here
//! too. The host supplies presentation, validation, and diagnostic policy.

mod execution;
mod input;
mod metadata;

pub use input::{ApiCallInput, ApiExplainInput, ApiSchemaInput, ApiSearchInput};
pub use metadata::{ToolDefinition, ToolKind, ToolSet};

use execution::tool_input_error;
pub use execution::{Continuation, Effect, IoResult, Policy, resume};
pub use execution::{FileEncoding, FileRead, FileReadResult, FileReference};
pub(super) use execution::{process_api_response, validate_file_path};

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

/// Dispatch a resolved API tool operation using the supplied host policy.
#[must_use]
pub fn dispatch(
    kind: ToolKind,
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
    policy: &Policy,
) -> Effect {
    match kind {
        ToolKind::Search => Effect::Done(search(arguments, spec)),
        ToolKind::Explain => Effect::Done(explain(arguments, spec, policy)),
        ToolKind::Call => call(arguments, spec, policy),
        ToolKind::Schema => Effect::Done(schema(arguments, spec, policy)),
    }
}

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
    policy: &Policy,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiExplainInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![ContentBlock::text(e.message)])),
    };
    let mut detail = match spec.explain(&input.endpoint) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{e}"
            ))]));
        }
    };

    (policy.present_endpoint)(&mut detail, spec);
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
    policy: &Policy,
) -> Result<CallToolResult, ErrorData> {
    let input: ApiSchemaInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return Ok(CallToolResult::error(vec![ContentBlock::text(e.message)])),
    };
    let mut detail = match spec.lookup_schema(&input.schema) {
        Ok(d) => d,
        Err(e) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{e}"
            ))]));
        }
    };

    (policy.present_schema)(&mut detail, spec);
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
            present_endpoint: |_, _| {},
            present_schema: |_, _| {},
            validate_body,
            validate_request: |_| Ok(()),
            append_error_details: |_, _| {},
        }
    }

    const DOCUMENT_TOOL_NAMES: [&str; 4] = [
        "catalog_find",
        "endpoint_details",
        "run_request",
        "type_details",
    ];

    fn document_tool_definitions(names: [&str; 4]) -> [ToolDefinition<'_>; 4] {
        [
            ToolDefinition {
                kind: ToolKind::Search,
                name: names[0],
                description: "Find document endpoints, then use endpoint_details.",
                input_description: Some("Document catalog search."),
                field_descriptions: &[("query", "Words to find in the document catalog.")],
            },
            ToolDefinition {
                kind: ToolKind::Explain,
                name: names[1],
                description: "Explain an endpoint found with catalog_find.",
                input_description: None,
                field_descriptions: &[],
            },
            ToolDefinition {
                kind: ToolKind::Call,
                name: names[2],
                description: "Invoke a document endpoint.",
                input_description: None,
                field_descriptions: &[(
                    "body",
                    "Use endpoint_details to inspect the request schema.",
                )],
            },
            ToolDefinition {
                kind: ToolKind::Schema,
                name: names[3],
                description: "Look up a document schema.",
                input_description: None,
                field_descriptions: &[],
            },
        ]
    }

    #[test]
    fn configured_metadata_owns_names_and_preserves_input_contracts() {
        let tools = {
            let names = DOCUMENT_TOOL_NAMES.map(str::to_owned);
            let mut definitions = document_tool_definitions(names.each_ref().map(String::as_str));
            definitions.swap(0, 2);
            ToolSet::new(&definitions).expect("valid document tool definitions")
        };
        let advertised = tools.list();
        let names: Vec<_> = advertised.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "run_request",
                "endpoint_details",
                "catalog_find",
                "type_details"
            ]
        );
        let metadata = serde_json::to_value(advertised).expect("serializable tool metadata");
        assert!(!metadata.to_string().to_lowercase().contains("onshape"));
        assert_eq!(
            metadata[2]["description"],
            "Find document endpoints, then use endpoint_details."
        );
        assert_eq!(
            metadata[2]["inputSchema"]["description"],
            "Document catalog search."
        );
        assert_eq!(
            metadata[2]["inputSchema"]["properties"]["query"]["description"],
            "Words to find in the document catalog."
        );
        assert_eq!(
            metadata[2]["inputSchema"]["properties"]["query"]["type"],
            "string"
        );
        assert_eq!(metadata[2]["inputSchema"]["required"], json!(["query"]));
        assert_eq!(metadata[0]["inputSchema"]["required"], json!(["endpoint"]));
        assert_eq!(
            metadata[0]["inputSchema"]["properties"]["body"]["description"],
            "Use endpoint_details to inspect the request schema."
        );
        for (index, tool) in metadata.as_array().expect("tool list").iter().enumerate() {
            assert_eq!(tool["annotations"]["readOnlyHint"], index != 0);
            assert_eq!(tool["annotations"]["destructiveHint"], index == 0);
        }
        assert_eq!(tools.resolve("onshape_api_call"), None);
        assert_eq!(tools.resolve("unknown_tool"), None);
    }

    #[test]
    fn configured_names_dispatch_all_four_operations() {
        let tools = ToolSet::new(&document_tool_definitions(DOCUMENT_TOOL_NAMES))
            .expect("valid document tool definitions");
        let spec = document_store_spec();
        let host_policy = policy(|_, _, _| Ok(()));
        for (name, arguments, expected_field, expected_value) in [
            (
                "catalog_find",
                json!({ "query": "createDocument" }),
                "/0/operation_id",
                "createDocument",
            ),
            (
                "endpoint_details",
                json!({ "endpoint": "createDocument" }),
                "/path",
                "/documents",
            ),
            (
                "type_details",
                json!({ "schema": "Document" }),
                "/name",
                "Document",
            ),
        ] {
            let kind = tools.resolve(name).expect("configured tool name");
            let Effect::Done(Ok(result)) =
                dispatch(kind, arguments.as_object(), &spec, &host_policy)
            else {
                panic!("catalog tools should complete without I/O");
            };
            assert_ne!(result.is_error, Some(true));
            let text = &result.content[0].as_text().expect("JSON text content").text;
            let value: Value = serde_json::from_str(text).expect("valid JSON");
            assert_eq!(value.pointer(expected_field), Some(&json!(expected_value)));
        }

        let arguments = json!({ "endpoint": "createDocument", "body": { "title": "Example" } });
        let kind = tools.resolve("run_request").expect("configured call tool");
        let Effect::ApiRequest { request, .. } =
            dispatch(kind, arguments.as_object(), &spec, &host_policy)
        else {
            panic!("the configured call tool should prepare an API request");
        };
        assert_eq!(request.method, http::Method::POST);
        assert_eq!(request.path, "/documents");
        assert_eq!(
            request.body.as_ref().and_then(RequestBody::as_json),
            Some(&json!({ "title": "Example" }))
        );
    }

    #[test]
    fn tool_configuration_rejects_ambiguous_or_blank_names() {
        let definitions = document_tool_definitions(DOCUMENT_TOOL_NAMES);
        let mut duplicate_name = definitions;
        duplicate_name[1].name = duplicate_name[0].name;
        assert_eq!(
            ToolSet::new(&duplicate_name).expect_err("duplicate name"),
            "duplicate API tool name: catalog_find"
        );

        let mut duplicate_kind = definitions;
        duplicate_kind[1].kind = ToolKind::Search;
        assert_eq!(
            ToolSet::new(&duplicate_kind).expect_err("duplicate operation leaves one missing"),
            "duplicate API tool kind: Search"
        );

        for name in ["", " \t\n"] {
            let mut blank_name = definitions;
            blank_name[1].name = name;
            assert_eq!(
                ToolSet::new(&blank_name).expect_err("blank name"),
                "API tool name must not be blank"
            );
        }
    }

    #[test]
    fn tool_configuration_rejects_names_outside_mcp_guidance() {
        let too_long = "a".repeat(129);
        for name in [
            " tool",
            "tool ",
            "two words",
            "tool\tname",
            "tool\nname",
            "tool/name",
            "tool,name",
            "tool:name",
            "café",
            "\0name",
            &too_long,
        ] {
            let mut definitions = document_tool_definitions(DOCUMENT_TOOL_NAMES);
            definitions[0].name = name;
            assert_eq!(
                ToolSet::new(&definitions).expect_err("invalid tool name"),
                format!(
                    "invalid API tool name {name:?}: expected 1-128 ASCII letters, digits, underscores, hyphens, or periods"
                )
            );
        }
    }

    #[test]
    fn tool_configuration_accepts_mcp_name_boundaries_and_distinct_case() {
        let longest = "a".repeat(128);
        let names = ["a", "A", "Admin.tools-v2_0", &longest];
        let tools = ToolSet::new(&document_tool_definitions(names))
            .expect("valid names at the length boundaries and with all allowed character classes");
        for ((name, kind), advertised) in names
            .into_iter()
            .zip([
                ToolKind::Search,
                ToolKind::Explain,
                ToolKind::Call,
                ToolKind::Schema,
            ])
            .zip(tools.list())
        {
            assert_eq!(advertised.name, name);
            assert_eq!(tools.resolve(name), Some(kind));
        }
    }

    #[test]
    fn tool_configuration_rejects_unknown_description_fields() {
        let mut definitions = document_tool_definitions(DOCUMENT_TOOL_NAMES);
        definitions[0].field_descriptions = &[("missing_field", "Unknown field name.")];
        assert_eq!(
            ToolSet::new(&definitions).expect_err("invalid description override"),
            "invalid API input description field \"missing_field\" for Search"
        );
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
                "components": {
                    "schemas": {
                        "Document": {
                            "type": "object",
                            "properties": { "title": { "type": "string" } }
                        }
                    }
                },
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
