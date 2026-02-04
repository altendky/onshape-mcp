//! Pure MCP protocol logic for Onshape integration.
//!
//! This crate contains sans-IO business logic with no async runtime dependencies.
//! All I/O operations are handled by the `onshape-mcp-io` crate.

use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};

/// The iconic Onshape regeneration success message.
pub const CATCH_PHRASE: &str =
    "Model regeneration complete. No rebuild errors. All features resolved.";

/// Creates the server info for MCP initialization.
///
/// # Arguments
///
/// * `name` - The server name (typically from `CARGO_PKG_NAME`)
/// * `version` - The server version (typically from `CARGO_PKG_VERSION`)
#[must_use]
pub fn server_info(name: &str, version: &str) -> ServerInfo {
    ServerInfo {
        capabilities: ServerCapabilities::builder().enable_tools().build(),
        server_info: Implementation {
            name: name.into(),
            version: version.into(),
            ..Default::default()
        },
        instructions: Some(format!(
            "Onshape MCP server for CAD integration. {CATCH_PHRASE}"
        )),
        ..Default::default()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn server_info_sets_name_and_version() {
        let info = server_info("test-server", "1.2.3");

        assert_eq!(info.server_info.name, "test-server");
        assert_eq!(info.server_info.version, "1.2.3");
    }

    #[test]
    fn server_info_enables_tools_capability() {
        let info = server_info("test", "0.0.0");

        assert!(info.capabilities.tools.is_some());
    }

    #[test]
    fn server_info_includes_instructions() {
        let info = server_info("test", "0.0.0");

        let instructions = info.instructions.expect("instructions should be set");
        assert!(instructions.contains("Onshape MCP server"));
        assert!(instructions.contains(CATCH_PHRASE));
    }
}
