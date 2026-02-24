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

use crate::config::ResolvedAuth;

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
    /// Build an auth status result from the resolved auth state.
    ///
    /// This is the primary constructor — it maps the core's [`ResolvedAuth`]
    /// to a user-facing status result. The `now` parameter is needed to
    /// determine whether OAuth tokens have expired.
    #[must_use]
    pub fn from_resolved(resolved: &ResolvedAuth, now: DateTime<Utc>) -> Self {
        match resolved {
            ResolvedAuth::NotConfigured {
                configured_method,
                detail,
            } => Self {
                status: AuthStatus::NotConfigured,
                auth_method: *configured_method,
                last_check: None,
                message: Some(detail.clone()),
            },
            ResolvedAuth::Basic => Self {
                status: AuthStatus::NotValidated,
                auth_method: AuthMethod::Basic,
                last_check: None,
                message: Some(
                    "Credentials configured but not yet validated against Onshape API".into(),
                ),
            },
            ResolvedAuth::OAuthReady { expires_at } => {
                if expires_at.is_some_and(|ea| ea <= now) {
                    Self {
                        status: AuthStatus::Expired,
                        auth_method: AuthMethod::OAuth,
                        last_check: None,
                        message: Some(
                            "OAuth access token has expired. \
                             Token refresh will be attempted on next API call."
                                .into(),
                        ),
                    }
                } else {
                    Self {
                        status: AuthStatus::NotValidated,
                        auth_method: AuthMethod::OAuth,
                        last_check: None,
                        message: Some(
                            "OAuth access token present but not yet validated against Onshape API"
                                .into(),
                        ),
                    }
                }
            }
            ResolvedAuth::OAuthPending => Self {
                status: AuthStatus::NotConfigured,
                auth_method: AuthMethod::OAuth,
                last_check: None,
                message: Some(
                    "OAuth client credentials configured but no access token present. \
                     Complete the OAuth authorization flow to obtain tokens."
                        .into(),
                ),
            },
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
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0)
            .single()
            .expect("valid datetime")
    }

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

    // ====================================================================
    // AuthStatusResult::from_resolved Tests
    // ====================================================================

    #[test]
    fn from_resolved_not_configured_basic() {
        let resolved = ResolvedAuth::NotConfigured {
            configured_method: AuthMethod::Basic,
            detail: "No credentials configured".into(),
        };
        let result = AuthStatusResult::from_resolved(&resolved, now());

        assert_eq!(result.status, AuthStatus::NotConfigured);
        assert_eq!(result.auth_method, AuthMethod::Basic);
        assert!(result.last_check.is_none());
        assert_eq!(result.message.as_deref(), Some("No credentials configured"));
    }

    #[test]
    fn from_resolved_not_configured_auto() {
        let resolved = ResolvedAuth::NotConfigured {
            configured_method: AuthMethod::Auto,
            detail: "No complete credentials found. Missing: API keys".into(),
        };
        let result = AuthStatusResult::from_resolved(&resolved, now());

        assert_eq!(result.status, AuthStatus::NotConfigured);
        assert_eq!(result.auth_method, AuthMethod::Auto);
    }

    #[test]
    fn from_resolved_not_configured_serializes() {
        let resolved = ResolvedAuth::NotConfigured {
            configured_method: AuthMethod::Basic,
            detail: "No credentials configured".into(),
        };
        let result = AuthStatusResult::from_resolved(&resolved, now());
        let json = serde_json::to_string(&result).expect("should serialize");

        assert!(json.contains("\"status\":\"not_configured\""));
        assert!(json.contains("\"auth_method\":\"basic\""));
    }

    #[test]
    fn from_resolved_basic() {
        let result = AuthStatusResult::from_resolved(&ResolvedAuth::Basic, now());

        assert_eq!(result.status, AuthStatus::NotValidated);
        assert_eq!(result.auth_method, AuthMethod::Basic);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("not yet validated"))
        );
    }

    #[test]
    fn from_resolved_basic_serializes() {
        let result = AuthStatusResult::from_resolved(&ResolvedAuth::Basic, now());
        let json = serde_json::to_string(&result).expect("should serialize");

        assert!(json.contains("\"status\":\"not_validated\""));
    }

    #[test]
    fn from_resolved_oauth_ready_not_expired() {
        let future = now() + chrono::Duration::hours(1);
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(future),
        };
        let result = AuthStatusResult::from_resolved(&resolved, now());

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
    fn from_resolved_oauth_ready_expired() {
        let past = now() - chrono::Duration::hours(1);
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(past),
        };
        let result = AuthStatusResult::from_resolved(&resolved, now());

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
    fn from_resolved_oauth_ready_no_expiry() {
        let resolved = ResolvedAuth::OAuthReady { expires_at: None };
        let result = AuthStatusResult::from_resolved(&resolved, now());

        assert_eq!(result.status, AuthStatus::NotValidated);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
    }

    #[test]
    fn from_resolved_oauth_pending() {
        let result = AuthStatusResult::from_resolved(&ResolvedAuth::OAuthPending, now());

        assert_eq!(result.status, AuthStatus::NotConfigured);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("no access token"))
        );
    }

    #[test]
    fn from_resolved_oauth_pending_serializes() {
        let result = AuthStatusResult::from_resolved(&ResolvedAuth::OAuthPending, now());
        let json = serde_json::to_string(&result).expect("should serialize");

        assert!(json.contains("\"auth_method\":\"oauth\""));
        assert!(json.contains("\"status\":\"not_configured\""));
    }
}
