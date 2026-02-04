//! I/O layer for the Onshape MCP server.
//!
//! This crate provides the async runtime integration and MCP transport handling.
//! It delegates business logic to `onshape-mcp-core`.

use rmcp::{ServerHandler, ServiceExt, model::ServerInfo, transport::stdio};

/// The MCP server handler for Onshape integration.
#[derive(Clone)]
pub struct OnshapeMcpServer {
    info: ServerInfo,
}

impl OnshapeMcpServer {
    /// Creates a new server instance.
    #[must_use]
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            info: onshape_mcp_core::server_info(name, version),
        }
    }
}

impl ServerHandler for OnshapeMcpServer {
    fn get_info(&self) -> ServerInfo {
        self.info.clone()
    }
}

/// Runs the MCP server on stdio transport.
///
/// # Arguments
///
/// * `name` - The server name (typically from `CARGO_PKG_NAME`)
/// * `version` - The server version (typically from `CARGO_PKG_VERSION`)
///
/// # Errors
///
/// Returns an error if the server fails to start or encounters a fatal error.
pub async fn run(name: &str, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let server = OnshapeMcpServer::new(name, version);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
