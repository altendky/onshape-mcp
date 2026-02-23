//! Pure MCP protocol logic for Onshape integration.
//!
//! This crate contains sans-IO business logic with no async runtime dependencies.
//! All I/O operations are handled by the `onshape-mcp-io` crate.

pub mod config;
pub mod openapi;
pub mod tools;

use chrono::{DateTime, Utc};
use onshape_client_core::auth::AuthMethod;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Authentication status for the Onshape API connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    /// Credentials are valid and working.
    Valid,
    /// Credentials are invalid (wrong key/secret).
    Invalid,
    /// Credentials have expired.
    Expired,
    /// No credentials have been configured.
    NotConfigured,
    /// Credentials are configured but have not been validated against the API.
    NotValidated,
}

/// Result of checking authentication status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthStatusResult {
    /// Current authentication status.
    pub status: AuthStatus,
    /// Configured authentication method.
    pub auth_method: AuthMethod,
    /// Timestamp of the last authentication check, if any.
    pub last_check: Option<DateTime<Utc>>,
    /// Human-readable message explaining the status.
    pub message: Option<String>,
}

impl AuthStatusResult {
    /// Returns a result indicating no credentials are configured.
    #[must_use]
    pub fn not_configured(auth_method: AuthMethod) -> Self {
        Self {
            status: AuthStatus::NotConfigured,
            auth_method,
            last_check: None,
            message: Some("No credentials configured".into()),
        }
    }

    /// Returns a result indicating credentials are configured but not yet validated.
    #[must_use]
    pub fn not_validated(auth_method: AuthMethod) -> Self {
        Self {
            status: AuthStatus::NotValidated,
            auth_method,
            last_check: None,
            message: Some(
                "Credentials configured but not yet validated against Onshape API".into(),
            ),
        }
    }

    /// Returns a result indicating only partial credentials are configured.
    #[must_use]
    pub fn partial_credentials(missing_field: &str, auth_method: AuthMethod) -> Self {
        Self {
            status: AuthStatus::NotConfigured,
            auth_method,
            last_check: None,
            message: Some(format!(
                "Incomplete credentials: {missing_field} is not configured"
            )),
        }
    }

    /// Returns a result indicating OAuth is configured but no token file is present.
    #[must_use]
    pub fn oauth_not_configured() -> Self {
        Self {
            status: AuthStatus::NotConfigured,
            auth_method: AuthMethod::OAuth,
            last_check: None,
            message: Some("OAuth client credentials configured but no access token present. Run the OAuth authorization flow to obtain tokens.".into()),
        }
    }

    /// Returns a result indicating OAuth tokens are present (not yet validated).
    #[must_use]
    pub fn oauth_token_present() -> Self {
        Self {
            status: AuthStatus::NotValidated,
            auth_method: AuthMethod::OAuth,
            last_check: None,
            message: Some(
                "OAuth access token present but not yet validated against Onshape API".into(),
            ),
        }
    }

    /// Returns a result indicating the OAuth access token has expired.
    #[must_use]
    pub fn oauth_token_expired() -> Self {
        Self {
            status: AuthStatus::Expired,
            auth_method: AuthMethod::OAuth,
            last_check: None,
            message: Some(
                "OAuth access token has expired. Re-run the OAuth authorization flow to obtain new tokens.".into(),
            ),
        }
    }
}

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

    #[test]
    fn auth_status_result_not_configured() {
        let result = AuthStatusResult::not_configured(AuthMethod::Basic);

        assert_eq!(result.status, AuthStatus::NotConfigured);
        assert_eq!(result.auth_method, AuthMethod::Basic);
        assert!(result.last_check.is_none());
        assert_eq!(result.message.as_deref(), Some("No credentials configured"));
    }

    #[test]
    fn auth_status_serializes_to_snake_case() {
        let result = AuthStatusResult::not_configured(AuthMethod::Basic);
        let json = serde_json::to_string(&result).expect("should serialize");

        assert!(json.contains("\"status\":\"not_configured\""));
        assert!(json.contains("\"auth_method\":\"basic\""));
    }

    #[test]
    fn auth_status_not_validated() {
        let result = AuthStatusResult::not_validated(AuthMethod::Basic);

        assert_eq!(result.status, AuthStatus::NotValidated);
        assert_eq!(result.auth_method, AuthMethod::Basic);
        assert!(result.last_check.is_none());
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("not yet validated"))
        );
    }

    #[test]
    fn auth_status_not_validated_serializes() {
        let result = AuthStatusResult::not_validated(AuthMethod::Basic);
        let json = serde_json::to_string(&result).expect("should serialize");

        assert!(json.contains("\"status\":\"not_validated\""));
    }

    #[test]
    fn auth_status_partial_credentials() {
        let result = AuthStatusResult::partial_credentials("secret_key", AuthMethod::Basic);

        assert_eq!(result.status, AuthStatus::NotConfigured);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("secret_key"))
        );
    }

    #[test]
    fn auth_status_oauth_not_configured() {
        let result = AuthStatusResult::oauth_not_configured();

        assert_eq!(result.status, AuthStatus::NotConfigured);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("OAuth") && m.contains("no access token"))
        );
    }

    #[test]
    fn auth_status_oauth_token_present() {
        let result = AuthStatusResult::oauth_token_present();

        assert_eq!(result.status, AuthStatus::NotValidated);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("not yet validated"))
        );
    }

    #[test]
    fn auth_status_oauth_token_expired() {
        let result = AuthStatusResult::oauth_token_expired();

        assert_eq!(result.status, AuthStatus::Expired);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("expired"))
        );
    }

    #[test]
    fn auth_status_oauth_serializes_correctly() {
        let result = AuthStatusResult::oauth_not_configured();
        let json = serde_json::to_string(&result).expect("should serialize");

        assert!(json.contains("\"auth_method\":\"oauth\""));
        assert!(json.contains("\"status\":\"not_configured\""));
    }
}
