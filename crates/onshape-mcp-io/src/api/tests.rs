#![allow(clippy::expect_used)]

use std::path::Path;

use onshape_mcp_core::tools::api::{Continuation, FileEncoding, FileReference};
use onshape_openapi::request::RequestBody;
use serde_json::{Value, json};

use super::*;

const POLICY: Policy = Policy {
    present_endpoint: |_, _| {},
    present_schema: |_, _| {},
    validate_body: |_, _, _| Ok(()),
    validate_request: |_| Ok(()),
    append_error_details: |_, _| {},
};

#[derive(Default)]
struct FakeExecutor {
    requests: Vec<ApiRequest>,
    outcome: Option<Result<RequestOutcome, ErrorData>>,
}

impl RequestExecutor for FakeExecutor {
    fn execute(
        &mut self,
        request: ApiRequest,
    ) -> impl Future<Output = Result<RequestOutcome, ErrorData>> + Send {
        self.requests.push(request);
        std::future::ready(self.outcome.take().expect("unexpected request execution"))
    }
}

fn request() -> ApiRequest {
    ApiRequest {
        method: http::Method::PATCH,
        path: "/items/a%2Fb".into(),
        query_params: vec![
            ("tag".into(), "first".into()),
            ("tag".into(), "second".into()),
        ],
        headers: http::HeaderMap::from_iter([(
            http::header::ACCEPT,
            http::HeaderValue::from_static("application/octet-stream"),
        )]),
        body: Some(RequestBody::Json(json!({"title": "Example"}))),
        content_type: Some("application/json".into()),
    }
}

fn request_effect() -> Effect {
    Effect::ApiRequest {
        request: request(),
        continuation: Continuation::FormatApiResponse,
    }
}

fn file_effect(path: &Path) -> Effect {
    Effect::ReadFiles {
        reads: vec![FileRead {
            path: path.to_path_buf(),
        }],
        continuation: Continuation::InjectFilesIntoRequest {
            request: request(),
            file_refs: vec![FileReference {
                path: path.to_str().expect("UTF-8 test path").into(),
                field: "content".into(),
                encoding: FileEncoding::TextUtf8,
            }],
        },
    }
}

fn text(result: &CallToolResult) -> &str {
    &result.content[0].as_text().expect("text result").text
}

#[tokio::test]
async fn files_are_injected_before_execution_and_binary_responses_are_preserved() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("content.txt");
    tokio::fs::write(&path, b"Uploaded content")
        .await
        .expect("write test file");
    let mut executor = FakeExecutor {
        outcome: Some(Ok(RequestOutcome::Response(Response {
            status: 200,
            headers: vec![("content-type".into(), "application/octet-stream".into())],
            body: vec![0, 255],
        }))),
        ..FakeExecutor::default()
    };

    let result = run(
        file_effect(&path),
        &POLICY,
        &mut executor,
        FileReadPolicy::Allow,
    )
    .await
    .expect("completed call");

    assert_eq!(executor.requests.len(), 1);
    let executed = &executor.requests[0];
    let expected = request();
    assert_eq!(executed.method, expected.method);
    assert_eq!(executed.path, expected.path);
    assert_eq!(executed.query_params, expected.query_params);
    assert_eq!(executed.headers, expected.headers);
    assert_eq!(executed.content_type, expected.content_type);
    assert_eq!(
        executed.body.as_ref().and_then(RequestBody::as_json),
        Some(&json!({"title": "Example", "content": "Uploaded content"}))
    );
    assert_ne!(result.is_error, Some(true));
    let body: Value = serde_json::from_str(text(&result)).expect("binary response metadata");
    assert_eq!(body["encoding"], "base64");
    assert_eq!(body["body"], "AP8=");
}

#[tokio::test]
async fn denied_reads_return_host_wording_before_file_errors_or_requests() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut executor = FakeExecutor::default();
    let result = run(
        file_effect(&dir.path().join("missing.txt")),
        &POLICY,
        &mut executor,
        FileReadPolicy::Deny("Local uploads are disabled by this host."),
    )
    .await
    .expect("tool error");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "Local uploads are disabled by this host.");
    assert!(executor.requests.is_empty());
}

#[tokio::test]
async fn read_failures_stop_before_request_execution() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut executor = FakeExecutor::default();
    let result = run(
        file_effect(&dir.path().join("missing.txt")),
        &POLICY,
        &mut executor,
        FileReadPolicy::Allow,
    )
    .await
    .expect("tool error");
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("failed to read file:"));
    assert!(executor.requests.is_empty());
}

#[tokio::test]
async fn host_policy_can_reject_injected_content_before_request_execution() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("content.txt");
    tokio::fs::write(&path, b"Rejected content")
        .await
        .expect("write test file");
    let policy = Policy {
        validate_request: |request| {
            assert_eq!(
                request.body.as_ref().and_then(RequestBody::as_json),
                Some(&json!({"title": "Example", "content": "Rejected content"}))
            );
            Err("Host rejected uploaded content.".into())
        },
        ..POLICY
    };
    let mut executor = FakeExecutor::default();
    let result = run(
        file_effect(&path),
        &policy,
        &mut executor,
        FileReadPolicy::Allow,
    )
    .await
    .expect("tool error");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "Host rejected uploaded content.");
    assert!(executor.requests.is_empty());
}

#[tokio::test]
async fn engine_and_executor_terminal_results_are_returned_unchanged() {
    for expected in [
        Ok(CallToolResult::error(vec![ContentBlock::text(
            "Host credentials are missing.",
        )])),
        Err(ErrorData::internal_error("Connection failed.", None)),
    ] {
        let mut executor = FakeExecutor::default();
        let result = run(
            Effect::Done(expected.clone()),
            &POLICY,
            &mut executor,
            FileReadPolicy::Allow,
        )
        .await;
        assert_eq!(
            serde_json::to_value(&result).expect("result"),
            serde_json::to_value(&expected).expect("expected result")
        );
        assert!(executor.requests.is_empty());

        executor.outcome = Some(expected.clone().map(RequestOutcome::ToolResult));
        let result = run(
            request_effect(),
            &POLICY,
            &mut executor,
            FileReadPolicy::Allow,
        )
        .await;
        assert_eq!(
            serde_json::to_value(result).expect("result"),
            serde_json::to_value(expected).expect("expected result")
        );
        assert_eq!(executor.requests.len(), 1);
    }
}

#[tokio::test]
async fn http_errors_retain_response_headers_and_host_diagnostics() {
    let mut executor = FakeExecutor {
        outcome: Some(Ok(RequestOutcome::Response(Response {
            status: 429,
            headers: vec![("retry-after".into(), "30".into())],
            body: vec![0, 255],
        }))),
        ..FakeExecutor::default()
    };
    let policy = Policy {
        append_error_details: |message, bytes| {
            assert_eq!(bytes, &[0, 255]);
            message.push_str("; host_code=BUSY");
        },
        ..POLICY
    };
    let result = run(
        request_effect(),
        &policy,
        &mut executor,
        FileReadPolicy::Allow,
    )
    .await
    .expect("tool error");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        text(&result),
        "API error (HTTP 429): category=rate_limited; transient=true; host_code=BUSY; retry_after_seconds=30"
    );
    assert_eq!(executor.requests.len(), 1);
}
