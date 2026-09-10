//! A local media API and test executor using only the public generic interfaces.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Multipart, Request, State},
    middleware::{self, Next},
    response::Response as HttpResponse,
    routing::{get, patch, post},
};
use http::{HeaderMap, Method, StatusCode, Uri, header};
use onshape_mcp_core::tools::api::{self, Policy, ToolDefinition, ToolKind, ToolSet};
use onshape_mcp_io::api::{FileReadPolicy, RequestExecutor, RequestOutcome, Response, run};
use onshape_openapi::{
    OpenApiSpec,
    request::{ApiRequest, RequestBody},
};
use rmcp::{ErrorData, model::CallToolResult};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};

pub const BINARY_CONTENT: &[u8] = &[0, 255, 10, 13, 128];
pub const TOOL_NAMES: [&str; 4] = ["media.find", "media.describe", "media.send", "media.type"];

const POLICY: Policy = Policy {
    validate_body: |_, _, _| Ok(()),
    validate_request: |_| Ok(()),
    append_error_details: |_, _| {},
};

/// Record the request as received by the server, before routing or body parsing.
#[derive(Clone, Debug)]
pub struct ObservedRequest {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
}

type Observations = Arc<Mutex<Vec<ObservedRequest>>>;

async fn observe(
    State(observations): State<Observations>,
    request: Request,
    next: Next,
) -> HttpResponse {
    observations.lock().await.push(ObservedRequest {
        method: request.method().clone(),
        uri: request.uri().clone(),
        headers: request.headers().clone(),
    });
    next.run(request).await
}

/// Echo parsed multipart parts so tests can check the bytes and field names.
async fn upload(mut multipart: Multipart) -> (StatusCode, Json<Value>) {
    let mut parts = Vec::new();
    while let Some(field) = multipart.next_field().await.expect("valid multipart field") {
        let name = field.name().expect("named form field").to_owned();
        let data = field.bytes().await.expect("complete form field");
        parts.push(json!({"name": name, "data": data.to_vec()}));
    }
    (StatusCode::CREATED, Json(json!(parts)))
}

fn tools() -> ToolSet {
    ToolSet::new(&[
        ToolDefinition {
            kind: ToolKind::Search,
            name: TOOL_NAMES[0],
            description: "Find media operations, then inspect them with media.describe.",
            input_description: None,
            field_descriptions: &[],
        },
        ToolDefinition {
            kind: ToolKind::Explain,
            name: TOOL_NAMES[1],
            description: "Explain a media operation found with media.find.",
            input_description: None,
            field_descriptions: &[],
        },
        ToolDefinition {
            kind: ToolKind::Call,
            name: TOOL_NAMES[2],
            description: "Send a media request described by media.describe.",
            input_description: None,
            field_descriptions: &[("body", "Use media.describe to inspect the request schema.")],
        },
        ToolDefinition {
            kind: ToolKind::Schema,
            name: TOOL_NAMES[3],
            description: "Inspect a media component schema.",
            input_description: None,
            field_descriptions: &[],
        },
    ])
    .expect("valid media tool configuration")
}

/// Test-only HTTP execution; deliberately independent of the Onshape client and auth.
struct HttpExecutor {
    client: reqwest::Client,
    base_url: String,
}

fn execution_error(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(format!("fixture HTTP execution failed: {error}"), None)
}

impl RequestExecutor for HttpExecutor {
    async fn execute(&mut self, request: ApiRequest) -> Result<RequestOutcome, ErrorData> {
        // OpenAPI paths begin with '/'; retain the server URL's path prefix.
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), request.path);
        let mut builder = self
            .client
            .request(request.method, url)
            .query(&request.query_params)
            .headers(request.headers);
        match request.body {
            Some(RequestBody::Json(body)) => {
                builder = builder
                    .header(
                        header::CONTENT_TYPE,
                        request
                            .content_type
                            .as_deref()
                            .unwrap_or("application/json"),
                    )
                    .body(serde_json::to_vec(&body).map_err(execution_error)?);
            }
            Some(RequestBody::Multipart(body)) => {
                let mut form = reqwest::multipart::Form::new();
                for (name, value) in body.text_fields {
                    form = form.text(name, value);
                }
                for field in body.binary_fields {
                    let mut part = reqwest::multipart::Part::bytes(field.data);
                    if let Some(content_type) = field.content_type {
                        part = part.mime_str(&content_type).map_err(execution_error)?;
                    }
                    form = form.part(field.field_name, part);
                }
                // Let reqwest supply the multipart boundary in Content-Type.
                builder = builder.multipart(form);
            }
            None => {}
        }
        let response = builder.send().await.map_err(execution_error)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let body = response.bytes().await.map_err(execution_error)?.to_vec();
        Ok(RequestOutcome::Response(Response {
            status,
            headers,
            body,
        }))
    }
}

/// Own the API server for one test and stop it even when an assertion fails.
pub struct Catalog {
    pub tools: ToolSet,
    spec: OpenApiSpec,
    executor: HttpExecutor,
    observations: Observations,
    server: JoinHandle<()>,
}

impl Catalog {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind media API");
        let address = listener.local_addr().expect("media API address");
        let observations = Observations::default();
        let app = Router::new()
            .route(
                "/api/v2/collections/{collection_id}/items/{item_id}",
                patch(|Json(body): Json<Value>| async move { Json(body) }),
            )
            .route("/api/v2/collections/{collection_id}/assets", post(upload))
            .route(
                "/api/v2/assets/{asset_id}/content",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/octet-stream")],
                        BINARY_CONTENT,
                    )
                }),
            )
            .route(
                "/api/v2/busy",
                get(|| async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [(header::RETRY_AFTER, "7")],
                        Json(json!({"code": "capacity"})),
                    )
                }),
            )
            .layer(middleware::from_fn_with_state(
                Arc::clone(&observations),
                observe,
            ));

        let mut document: Value =
            serde_json::from_str(include_str!("../fixtures/media-catalog.json"))
                .expect("media catalog fixture");
        document["servers"][0]["url"] = json!(format!("http://{address}/api/v2/"));
        let spec = OpenApiSpec::from_json(&document.to_string()).expect("parse media API");
        let executor = HttpExecutor {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("local HTTP client"),
            base_url: spec.server_url().to_owned(),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve media API");
        });
        Self {
            tools: tools(),
            spec,
            executor,
            observations,
            server,
        }
    }

    /// Resolve the configured name and run the public engine through to its MCP result.
    pub async fn invoke(
        &mut self,
        name: &str,
        arguments: Value,
        file_reads: FileReadPolicy<'_>,
    ) -> CallToolResult {
        let kind = self.tools.resolve(name).expect("configured media tool");
        let effect = api::dispatch(kind, arguments.as_object(), &self.spec, &POLICY);
        run(effect, &POLICY, &mut self.executor, file_reads)
            .await
            .expect("MCP result")
    }

    pub async fn requests(&self) -> Vec<ObservedRequest> {
        self.observations.lock().await.clone()
    }
}

impl Drop for Catalog {
    fn drop(&mut self) {
        self.server.abort();
    }
}
