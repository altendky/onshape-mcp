//! Exercise a non-Onshape API through public metadata, dispatch, and runner interfaces.

#![allow(clippy::expect_used)]

#[path = "support/media_catalog.rs"]
mod media_catalog;

use std::collections::HashMap;

use base64::Engine;
use http::{Method, header};
use onshape_mcp_io::api::FileReadPolicy;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use media_catalog::{BINARY_CONTENT, Catalog, TOOL_NAMES};

fn text(result: &CallToolResult) -> &str {
    &result.content[0].as_text().expect("text content").text
}

fn json_result(result: &CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "unexpected tool error: {}",
        text(result)
    );
    serde_json::from_str(text(result)).expect("JSON result")
}

#[tokio::test]
async fn catalog_tools_and_json_calls_work_with_alternate_names() {
    let mut catalog = Catalog::start().await;
    let advertised = catalog.tools.list();
    assert_eq!(
        advertised
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        TOOL_NAMES
    );
    let metadata = serde_json::to_string(&advertised).expect("tool metadata");
    assert!(!metadata.to_lowercase().contains("onshape"));

    let search = catalog
        .invoke(
            "media.find",
            json!({"query": "updateItem"}),
            FileReadPolicy::Deny("No local files."),
        )
        .await;
    let endpoints = json_result(&search);
    assert_eq!(endpoints.as_array().expect("search results").len(), 1);
    let endpoint = endpoints[0]["operation_id"].as_str().expect("operation ID");
    assert_eq!(endpoint, "updateItem");
    let explanation = catalog
        .invoke(
            "media.describe",
            json!({"endpoint": endpoint}),
            FileReadPolicy::Deny("No local files."),
        )
        .await;
    let explanation = json_result(&explanation);
    assert_eq!(explanation["method"], "PATCH");
    assert_eq!(
        explanation["path"],
        "/collections/{collection_id}/items/{item_id}"
    );
    assert_eq!(
        explanation["request_body_schema"]["properties"]["title"]["type"],
        "string"
    );
    let schema = catalog
        .invoke(
            "media.type",
            json!({"schema": "ItemPatch"}),
            FileReadPolicy::Deny("No local files."),
        )
        .await;
    let schema = json_result(&schema);
    assert_eq!(schema["name"], "ItemPatch");
    assert_eq!(schema["properties"]["title"]["type"], "string");
    assert_eq!(schema["required"], json!(["title"]));
    assert!(
        catalog.requests().await.is_empty(),
        "catalog tools must not execute HTTP requests"
    );

    let body = json!({"title": "A media item"});
    let result = catalog
        .invoke(
            "media.send",
            json!({
                "endpoint": endpoint,
                "path_params": {"collection_id": "shelf/one", "item_id": "item two"},
                "query_params": {"note": "a & b/+?"},
                "header_params": {"X-Workspace": "sandbox"},
                "body": body,
            }),
            FileReadPolicy::Deny("No local files."),
        )
        .await;
    assert_eq!(json_result(&result), body);
    let requests = catalog.requests().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::PATCH);
    assert_eq!(
        request.uri.path(),
        "/api/v2/collections/shelf%2Fone/items/item%20two"
    );
    let query: HashMap<_, _> =
        url::form_urlencoded::parse(request.uri.query().expect("query").as_bytes())
            .into_owned()
            .collect();
    assert_eq!(query, HashMap::from([("note".into(), "a & b/+?".into())]));
    assert_eq!(request.headers["x-workspace"], "sandbox");
    assert_eq!(request.headers[header::CONTENT_TYPE], "application/json");
    assert!(!request.headers.contains_key(header::AUTHORIZATION));
}

#[tokio::test]
async fn local_files_reach_the_api_as_binary_multipart_parts() {
    let mut catalog = Catalog::start().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("asset.bin");
    tokio::fs::write(&path, BINARY_CONTENT)
        .await
        .expect("write asset");
    let result = catalog
        .invoke(
            "media.send",
            json!({
                "endpoint": "uploadAsset",
                "path_params": {"collection_id": "shelf"},
                "body": {"label": "Sample asset"},
                "file_refs": [{"path": path, "field": "file", "encoding": "raw_bytes"}],
            }),
            FileReadPolicy::Allow,
        )
        .await;
    let parts = json_result(&result);
    let parts = parts.as_array().expect("parsed multipart parts");
    assert_eq!(parts.len(), 2);
    let fields: HashMap<_, _> = parts
        .iter()
        .map(|part| (part["name"].as_str().expect("part name"), &part["data"]))
        .collect();
    assert_eq!(fields["label"], &json!(b"Sample asset".as_slice()));
    assert_eq!(fields["file"], &json!(BINARY_CONTENT));
    let requests = catalog.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].uri.path(), "/api/v2/collections/shelf/assets");
    assert!(
        requests[0].headers[header::CONTENT_TYPE]
            .to_str()
            .expect("content type")
            .starts_with("multipart/form-data; boundary=")
    );
    assert!(!requests[0].headers.contains_key(header::AUTHORIZATION));
}

#[tokio::test]
async fn binary_and_error_responses_preserve_http_semantics() {
    let mut catalog = Catalog::start().await;
    let result = catalog
        .invoke(
            "media.send",
            json!({
                "endpoint": "downloadAsset", "path_params": {"asset_id": "asset-1"},
            }),
            FileReadPolicy::Deny("No local files."),
        )
        .await;
    let binary = json_result(&result);
    assert_eq!(binary["encoding"], "base64");
    assert_eq!(binary["contentType"], "application/octet-stream");
    assert_eq!(binary["status"], 200);
    assert_eq!(binary["byteLength"], BINARY_CONTENT.len());
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(binary["body"].as_str().expect("encoded bytes"))
            .expect("base64 response"),
        BINARY_CONTENT
    );

    let result = catalog
        .invoke(
            "media.send",
            json!({"endpoint": "checkCapacity"}),
            FileReadPolicy::Deny("No local files."),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        text(&result),
        "API error (HTTP 503): category=service_unavailable; transient=true; retry_after_seconds=7"
    );
    let requests = catalog.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].uri.path(), "/api/v2/assets/asset-1/content");
    assert_eq!(
        requests[0].headers[header::ACCEPT],
        "application/octet-stream"
    );
    assert_eq!(requests[1].uri.path(), "/api/v2/busy");
}

#[tokio::test]
async fn denied_file_reads_return_host_diagnostics_without_http() {
    let mut catalog = Catalog::start().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("missing.bin");
    let result = catalog
        .invoke(
            "media.send",
            json!({
                "endpoint": "uploadAsset",
                "path_params": {"collection_id": "shelf"},
                "body": {"label": "Not uploaded"},
                "file_refs": [{"path": path, "field": "file", "encoding": "raw_bytes"}],
            }),
            FileReadPolicy::Deny("Local uploads are disabled by this host."),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text(&result), "Local uploads are disabled by this host.");
    assert!(catalog.requests().await.is_empty());
}
