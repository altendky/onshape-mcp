//! I/O layer for the Onshape MCP server.
//!
//! This crate provides the async runtime integration and MCP transport handling.
//! It delegates all tool logic to `onshape-mcp-core` and HTTP execution to
//! `onshape-client-io`.

pub mod config;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
        ServerInfo,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
};
use secrecy::SecretString;

use onshape_client_core::auth::Credentials;
use onshape_client_io::{ClientConfig, OnshapeClient};
use onshape_mcp_core::config::{AppConfig, CredentialStatus};
use onshape_mcp_core::openapi::OpenApiSpec;
use onshape_mcp_core::tools::{self, ToolResult};

/// The embedded Onshape `OpenAPI` specification JSON.
///
/// Included at compile time from `onshape-openapi.json` in the crate root.
/// The spec is ~1.8 MB and adds to the binary size, but simplifies
/// distribution (single binary, no external files needed).
const OPENAPI_SPEC_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/onshape-openapi.json"));

/// The MCP server handler for Onshape integration.
///
/// Uses `Arc<AppConfig>` because `SecretString` (used for API keys)
/// intentionally does not implement `Clone` to prevent secret proliferation.
///
/// The `http_client` is `Some` when credentials are fully configured, and
/// `None` when they are missing or incomplete.
#[derive(Clone)]
pub struct OnshapeMcpServer {
    info: ServerInfo,
    config: Arc<AppConfig>,
    spec: Arc<OpenApiSpec>,
    http_client: Option<OnshapeClient>,
}

impl OnshapeMcpServer {
    /// Creates a new server instance.
    ///
    /// If credentials are fully configured, an HTTP client is created for
    /// executing API calls. Otherwise, API call requests will return an
    /// informative error.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedded `OpenAPI` spec fails to parse or
    /// the HTTP client fails to initialize.
    pub fn new(
        name: &str,
        version: &str,
        config: AppConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let spec = OpenApiSpec::from_json(OPENAPI_SPEC_JSON)?;

        let http_client = build_http_client(&config, spec.server_url())?;

        Ok(Self {
            info: onshape_mcp_core::server_info(name, version),
            config: Arc::new(config),
            spec: Arc::new(spec),
            http_client,
        })
    }
}

impl ServerHandler for OnshapeMcpServer {
    fn get_info(&self) -> ServerInfo {
        self.info.clone()
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        // Core returns Vec<Tool> directly - no conversion needed
        std::future::ready(Ok(ListToolsResult {
            tools: tools::list_tools(),
            next_cursor: None,
            meta: None,
        }))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Dispatch through core, then handle the effect
        let result = tools::call_tool(
            &request.name,
            request.arguments.as_ref(),
            &self.config.auth,
            Some(&self.spec),
        );

        match result {
            ToolResult::Immediate(r) => r,
            ToolResult::OnshapeApiRequest { request: api_req } => {
                execute_api_request(self.http_client.as_ref(), &api_req).await
            }
        }
    }
}

/// Execute an API request using the HTTP client, or return an error if
/// credentials are not configured.
async fn execute_api_request(
    http_client: Option<&OnshapeClient>,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<CallToolResult, McpError> {
    let Some(client) = http_client else {
        return Ok(CallToolResult {
            content: vec![Content::text(
                "Cannot execute API call: credentials are not configured. \
                 Set access_key and secret_key via config file, environment \
                 variables, or CLI flags.",
            )],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        });
    };

    match client.execute(api_req).await {
        Ok(response) => tools::process_api_response(response.status, &response.body),
        Err(e) => Ok(CallToolResult {
            content: vec![Content::text(format!("HTTP request failed: {e}"))],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        }),
    }
}

/// Build an HTTP client if credentials are fully configured.
///
/// Returns `Ok(None)` if credentials are missing (not an error — the server
/// can still serve search/explain tools). Returns `Err` if credentials are
/// present but the client fails to initialize.
fn build_http_client(
    config: &AppConfig,
    server_url: &str,
) -> Result<Option<OnshapeClient>, Box<dyn std::error::Error + Send + Sync>> {
    if config.auth.credential_status() != CredentialStatus::BothPresent {
        return Ok(None);
    }

    // Both keys are guaranteed present by the `BothPresent` check above.
    let (Some(access_key), Some(secret_key)) = (&config.auth.access_key, &config.auth.secret_key)
    else {
        // Safety: BothPresent guarantees both keys are present; this branch
        // is logically unreachable.
        unreachable!("credential_status() returned BothPresent but keys are None");
    };

    let credentials = Arc::new(Credentials {
        access_key: SecretString::from(access_key.clone()),
        secret_key: SecretString::from(secret_key.clone()),
    });

    let client_config = ClientConfig {
        base_url: server_url.to_string(),
        credentials,
        auth_method: config.auth.method,
        timeout: Some(config.http.timeout),
    };

    let client = OnshapeClient::new(client_config)?;
    Ok(Some(client))
}

/// Runs the MCP server on stdio transport.
///
/// # Arguments
///
/// * `name` - The server name (typically from `CARGO_PKG_NAME`)
/// * `version` - The server version (typically from `CARGO_PKG_VERSION`)
/// * `config` - Application configuration (loaded by the binary crate)
///
/// # Errors
///
/// Returns an error if the server fails to start or encounters a fatal error.
pub async fn run(
    name: &str,
    version: &str,
    config: AppConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = OnshapeMcpServer::new(name, version, config)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
