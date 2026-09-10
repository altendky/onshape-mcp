//! Verify the Onshape boundary around the generic API runner.

#![allow(clippy::expect_used)]

use onshape_mcp_core::{
    ValidationStatus,
    tools::{Continuation, FileEncoding, FileReference},
};
use onshape_openapi::request::{ApiRequest, RequestBody};
use serde_json::json;

use super::*;

fn request() -> ApiRequest {
    ApiRequest {
        method: http::Method::POST,
        path: "/documents".into(),
        query_params: vec![],
        headers: http::HeaderMap::new(),
        body: Some(RequestBody::Json(json!({}))),
        content_type: Some("application/json".into()),
    }
}

fn unconfigured() -> ApiState {
    ApiState::NotConfigured {
        configured_method: AuthMethod::Auto,
        detail: "test".into(),
    }
}

fn text(result: &CallToolResult) -> &str {
    &result.content[0].as_text().expect("text result").text
}

#[tokio::test]
async fn file_calls_preserve_transport_restrictions_and_onshape_validation_order() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("name.txt");
    for (allow_reads, contents, expected) in [
        (
            false,
            "Example",
            "File read operations are not supported over the HTTP transport.",
        ),
        (true, "", "createDocument name must not be blank"),
        (
            true,
            "Example",
            "Cannot execute API call: credentials are not configured.",
        ),
    ] {
        tokio::fs::write(&path, contents)
            .await
            .expect("write test name");
        let effect = ToolEffect::ReadFiles {
            reads: vec![tools::FileRead { path: path.clone() }],
            continuation: Continuation::InjectFilesIntoRequest {
                request: request(),
                file_refs: vec![FileReference {
                    path: path.to_str().expect("UTF-8 test path").into(),
                    field: "name".into(),
                    encoding: FileEncoding::TextUtf8,
                }],
            },
        };
        let validation = tokio::sync::Mutex::new(ValidationState::default());
        let result = dispatch_tool_effect(
            effect,
            &mut unconfigured(),
            &validation,
            None,
            false,
            allow_reads,
        )
        .await
        .expect("tool error");
        assert_eq!(result.is_error, Some(true));
        assert!(
            text(&result).contains(expected),
            "unexpected result: {}",
            text(&result)
        );
        assert_eq!(
            validation.lock().await.status,
            ValidationStatus::NotValidated
        );
    }
}

#[tokio::test]
async fn unavailable_credentials_complete_generic_and_host_calls_without_http() {
    for pending in [false, true] {
        for continuation in [
            Continuation::FormatApiResponse,
            Continuation::ProcessAuthValidation {
                resolved_auth: ResolvedAuth::Basic,
            },
        ] {
            let mut state = if pending {
                ApiState::OAuthPending(Box::new(OAuthPendingState {
                    refresh_method: PendingRefreshMethod::Proxy {
                        proxy_url: "https://proxy.example.com".into(),
                    },
                    base_url: "https://api.example.com".into(),
                    timeout: Duration::from_secs(1),
                    token_path: PathBuf::from("unused-tokens.json"),
                }))
            } else {
                unconfigured()
            };
            let validation = tokio::sync::Mutex::new(ValidationState::default());
            let effect = ToolEffect::ApiRequest {
                request: request(),
                continuation,
            };
            let result = dispatch_tool_effect(effect, &mut state, &validation, None, false, false)
                .await
                .expect("credential diagnostic");
            let expected = if pending {
                oauth_pending_error()
            } else {
                not_configured_error()
            };
            assert_eq!(
                serde_json::to_value(result).expect("result"),
                serde_json::to_value(expected).expect("expected result")
            );
            let validation = validation.lock().await.clone();
            assert_eq!(validation.status, ValidationStatus::NotValidated);
            assert!(validation.last_check.is_none());
        }
    }
}

#[tokio::test]
async fn generic_and_host_continuations_share_authenticated_execution_and_validation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test API");
    let address = listener.local_addr().expect("test API address");
    let app =
        axum::Router::new().route(
            "/status/{code}",
            axum::routing::get(
                |axum::extract::Path(status): axum::extract::Path<u16>,
                 headers: http::HeaderMap| async move {
                    assert_eq!(headers[http::header::AUTHORIZATION], "Bearer test-access");
                    (
                        http::StatusCode::from_u16(status).expect("test status"),
                        axum::Json(json!({"ok": true})),
                    )
                },
            ),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test API");
    });
    let client = OnshapeClient::new(ClientConfig {
        base_url: format!("http://{address}"),
        auth: ClientAuthConfig::Bearer {
            access_token: AccessToken::new("test-access".into()),
        },
        timeout: Some(Duration::from_secs(5)),
    })
    .expect("test client");
    let mut state = ApiState::Basic(client);
    for (status, expected) in [
        (200, ValidationStatus::Valid),
        (401, ValidationStatus::Invalid),
        (503, ValidationStatus::NotValidated),
    ] {
        for host_continuation in [false, true] {
            let validation = tokio::sync::Mutex::new(ValidationState::default());
            let mut request = request();
            request.method = http::Method::GET;
            request.path = format!("/status/{status}");
            request.body = None;
            request.content_type = None;
            let continuation = if host_continuation {
                Continuation::ProcessAuthValidation {
                    resolved_auth: ResolvedAuth::Basic,
                }
            } else {
                Continuation::FormatApiResponse
            };
            let result = dispatch_tool_effect(
                ToolEffect::ApiRequest {
                    request,
                    continuation,
                },
                &mut state,
                &validation,
                None,
                false,
                false,
            )
            .await
            .expect("completed request");
            let validation = validation.lock().await.clone();
            assert_eq!(validation.status, expected);
            assert_eq!(validation.last_check.is_some(), status != 503);
            if host_continuation {
                assert_ne!(result.is_error, Some(true));
                if status == 200 {
                    assert_eq!(
                        validation.message.as_deref(),
                        Some("Credentials validated successfully")
                    );
                }
            } else {
                assert_eq!(result.is_error, Some(status >= 400));
                if status == 200 {
                    assert!(validation.message.is_none());
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(text(&result))
                            .expect("JSON result"),
                        json!({"ok": true})
                    );
                }
            }
        }
    }
    server.abort();
}
