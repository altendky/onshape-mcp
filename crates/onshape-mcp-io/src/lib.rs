//! I/O layer for the Onshape MCP server.
//!
//! This crate provides the async runtime integration and MCP transport handling.
//! It delegates all tool logic to `onshape-mcp-core`.

pub mod config;
pub mod oauth;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
};

use onshape_mcp_core::config::AppConfig;
use onshape_mcp_core::tools;

/// The MCP server handler for Onshape integration.
///
/// Uses `Arc<AppConfig>` because `SecretString` (used for API keys)
/// intentionally does not implement `Clone` to prevent secret proliferation.
#[derive(Clone)]
pub struct OnshapeMcpServer {
    info: ServerInfo,
    config: Arc<AppConfig>,
}

impl OnshapeMcpServer {
    /// Creates a new server instance.
    #[must_use]
    pub fn new(name: &str, version: &str, config: AppConfig) -> Self {
        Self {
            info: onshape_mcp_core::server_info(name, version),
            config: Arc::new(config),
        }
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
        // Core returns Result<CallToolResult, ErrorData> directly - no conversion needed
        std::future::ready(tools::call_tool(
            &request.name,
            request.arguments.as_ref(),
            &self.config.auth,
        ))
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
    let server = OnshapeMcpServer::new(name, version, config);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
