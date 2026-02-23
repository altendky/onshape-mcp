//! I/O layer for the Onshape MCP server.
//!
//! This crate provides the async runtime integration and MCP transport handling.
//! It delegates all tool logic to `onshape-mcp-core`.

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

use onshape_mcp_core::config::AppConfig;
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
#[derive(Clone)]
pub struct OnshapeMcpServer {
    info: ServerInfo,
    config: Arc<AppConfig>,
    spec: Arc<OpenApiSpec>,
}

impl OnshapeMcpServer {
    /// Creates a new server instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedded `OpenAPI` spec fails to parse.
    pub fn new(
        name: &str,
        version: &str,
        config: AppConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let spec = OpenApiSpec::from_json(OPENAPI_SPEC_JSON)?;
        Ok(Self {
            info: onshape_mcp_core::server_info(name, version),
            config: Arc::new(config),
            spec: Arc::new(spec),
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

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        // Dispatch through core, then handle the effect
        let result = tools::call_tool(
            &request.name,
            request.arguments.as_ref(),
            &self.config.auth,
            Some(&self.spec),
        );

        std::future::ready(match result {
            ToolResult::Immediate(r) => r,
            ToolResult::OnshapeApiRequest { request: _api_req } => {
                // The HTTP client is not yet wired up. Return an informative error.
                let content = Content::text(
                    "The Onshape HTTP client is not yet implemented. \
                     The API request was validated successfully, but cannot be \
                     executed until the onshape-client-io crate is built. \
                     Use onshape_api_search and onshape_api_explain to explore \
                     available endpoints in the meantime.",
                );
                Ok(CallToolResult {
                    content: vec![content],
                    is_error: Some(true),
                    structured_content: None,
                    meta: None,
                })
            }
        })
    }
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
) -> Result<(), Box<dyn std::error::Error>> {
    let server = OnshapeMcpServer::new(name, version, config)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
