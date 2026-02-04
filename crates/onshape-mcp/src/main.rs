//! Onshape MCP Server
//!
//! A Model Context Protocol server for Onshape CAD integration.

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    onshape_mcp_io::run(NAME, VERSION).await
}
