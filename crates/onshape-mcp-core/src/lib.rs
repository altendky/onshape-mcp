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

/// Whether credentials have been confirmed working via an API call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    /// Credentials confirmed working via API call.
    Valid,
    /// Credentials confirmed invalid (API returned 401).
    Invalid,
    /// Credentials not yet validated against the API.
    NotValidated,
}

/// Runtime credential validation state.
///
/// Tracks whether credentials have been confirmed working against the
/// Onshape API. This is a runtime concern managed by the I/O layer,
/// separate from [`ResolvedAuth`] which represents credential configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidationState {
    /// Whether credentials have been validated.
    pub status: ValidationStatus,
    /// When the last validation check occurred.
    pub last_check: Option<DateTime<Utc>>,
    /// Human-readable detail about the validation result.
    pub message: Option<String>,
}

impl Default for ValidationState {
    fn default() -> Self {
        Self {
            status: ValidationStatus::NotValidated,
            last_check: None,
            message: None,
        }
    }
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
    /// Build an auth status result from resolved auth state and optional
    /// runtime validation state.
    ///
    /// This is the primary constructor — it maps the core's [`ResolvedAuth`]
    /// to a user-facing status result, then overlays any runtime validation
    /// information. The `now` parameter is needed to determine whether OAuth
    /// tokens have expired.
    ///
    /// Validation can override the status for states where credentials are
    /// present (`Basic`, `OAuthReady`).  `Expired` can also be overridden
    /// because the I/O layer proactively refreshes tokens before executing
    /// the validation request.  `NotConfigured` and `OAuthPending` take
    /// precedence over validation results because they represent structural
    /// issues where no credentials exist to validate.
    #[must_use]
    pub fn new(
        resolved: &ResolvedAuth,
        validation: Option<&ValidationState>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut result = match resolved {
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
        };

        // Overlay validation state for states where credentials are present.
        // NotConfigured and OAuthPending take precedence — those represent
        // structural issues that validation cannot override.
        //
        // Expired CAN be overridden by Valid: the I/O layer proactively
        // refreshes expired tokens before executing the validation request
        // (GET /users/sessioninfo).  If that request succeeds (200), the
        // token was refreshed and the session is valid — reporting Expired
        // would be misleading.
        if let Some(v) = validation {
            let can_override = matches!(
                result.status,
                AuthStatus::NotValidated | AuthStatus::Expired
            );
            if can_override {
                match v.status {
                    ValidationStatus::Valid => {
                        result.status = AuthStatus::Valid;
                        result.last_check = v.last_check;
                        result.message = Some(
                            v.message
                                .clone()
                                .unwrap_or_else(|| "Credentials validated successfully".into()),
                        );
                    }
                    ValidationStatus::Invalid => {
                        result.status = AuthStatus::Invalid;
                        result.last_check = v.last_check;
                        result.message = Some(
                            v.message
                                .clone()
                                .unwrap_or_else(|| "Credentials are invalid".into()),
                        );
                    }
                    ValidationStatus::NotValidated => {
                        // No change — keep the existing NotValidated status.
                    }
                }
            }
        }

        result
    }

    /// Build an auth status result from the resolved auth state alone.
    ///
    /// Convenience wrapper around [`Self::new`] that does not include
    /// runtime validation state. Useful when only the credential topology
    /// is known (e.g. during initial startup before any API calls).
    #[must_use]
    pub fn from_resolved(resolved: &ResolvedAuth, now: DateTime<Utc>) -> Self {
        Self::new(resolved, None, now)
    }
}

/// The iconic Onshape regeneration success message.
pub const CATCH_PHRASE: &str =
    "Model regeneration complete. No rebuild errors. All features resolved.";

/// The instructions text shared between the initialize response and the
/// `onshape_mcp_get_started` tool.
#[must_use]
pub fn instructions() -> String {
    format!(
        "Onshape MCP server for CAD integration. \
         This server provides insight resources with practical Onshape API \
         guidance. Before calling endpoints for the first time, list and \
         read relevant resources to avoid common pitfalls. \
         {CATCH_PHRASE}"
    )
}

/// Creates the server info for MCP initialization.
///
/// # Arguments
///
/// * `name` - The server name (typically from `CARGO_PKG_NAME`)
/// * `version` - The server version (typically from `CARGO_PKG_VERSION`)
#[must_use]
pub fn server_info(name: &str, version: &str) -> ServerInfo {
    ServerInfo::new(
        ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build(),
    )
    .with_server_info(Implementation::new(name, version))
    .with_instructions(instructions())
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
    fn server_info_enables_resources_capability() {
        let info = server_info("test", "0.0.0");

        assert!(info.capabilities.resources.is_some());
    }

    #[test]
    fn server_info_includes_instructions() {
        let info = server_info("test", "0.0.0");

        let instructions = info.instructions.expect("instructions should be set");
        assert!(instructions.contains("Onshape MCP server"));
        assert!(instructions.contains("insight resources"));
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

    // ====================================================================
    // ValidationState Tests
    // ====================================================================

    #[test]
    fn validation_state_default_is_not_validated() {
        let state = ValidationState::default();
        assert_eq!(state.status, ValidationStatus::NotValidated);
        assert!(state.last_check.is_none());
        assert!(state.message.is_none());
    }

    #[test]
    fn validation_state_serializes() {
        let state = ValidationState {
            status: ValidationStatus::Valid,
            last_check: Some(now()),
            message: Some("ok".into()),
        };
        let json = serde_json::to_string(&state).expect("should serialize");
        assert!(json.contains("\"status\":\"valid\""));
        assert!(json.contains("\"message\":\"ok\""));
    }

    #[test]
    fn validation_state_deserializes() {
        let json = r#"{"status":"invalid","last_check":null,"message":"bad"}"#;
        let state: ValidationState = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(state.status, ValidationStatus::Invalid);
        assert!(state.last_check.is_none());
        assert_eq!(state.message.as_deref(), Some("bad"));
    }

    // ====================================================================
    // AuthStatusResult::new() Tests
    // ====================================================================

    fn valid_validation() -> ValidationState {
        ValidationState {
            status: ValidationStatus::Valid,
            last_check: Some(now()),
            message: Some("Credentials validated successfully".into()),
        }
    }

    fn invalid_validation() -> ValidationState {
        ValidationState {
            status: ValidationStatus::Invalid,
            last_check: Some(now()),
            message: Some("API returned 401 Unauthorized".into()),
        }
    }

    fn not_validated_validation() -> ValidationState {
        ValidationState::default()
    }

    #[test]
    fn new_basic_with_valid_validation_overrides_to_valid() {
        let result = AuthStatusResult::new(&ResolvedAuth::Basic, Some(&valid_validation()), now());
        assert_eq!(result.status, AuthStatus::Valid);
        assert!(result.last_check.is_some());
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|m| m.contains("validated"))
        );
    }

    #[test]
    fn new_basic_with_invalid_validation_overrides_to_invalid() {
        let result =
            AuthStatusResult::new(&ResolvedAuth::Basic, Some(&invalid_validation()), now());
        assert_eq!(result.status, AuthStatus::Invalid);
        assert!(result.last_check.is_some());
        assert!(result.message.as_deref().is_some_and(|m| m.contains("401")));
    }

    #[test]
    fn new_basic_with_not_validated_keeps_not_validated() {
        let result = AuthStatusResult::new(
            &ResolvedAuth::Basic,
            Some(&not_validated_validation()),
            now(),
        );
        assert_eq!(result.status, AuthStatus::NotValidated);
    }

    #[test]
    fn new_basic_with_none_validation_is_not_validated() {
        let result = AuthStatusResult::new(&ResolvedAuth::Basic, None, now());
        assert_eq!(result.status, AuthStatus::NotValidated);
    }

    #[test]
    fn new_oauth_ready_with_valid_validation_overrides() {
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(now() + chrono::Duration::hours(1)),
        };
        let result = AuthStatusResult::new(&resolved, Some(&valid_validation()), now());
        assert_eq!(result.status, AuthStatus::Valid);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
    }

    #[test]
    fn new_oauth_ready_with_invalid_validation_overrides() {
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(now() + chrono::Duration::hours(1)),
        };
        let result = AuthStatusResult::new(&resolved, Some(&invalid_validation()), now());
        assert_eq!(result.status, AuthStatus::Invalid);
        assert_eq!(result.auth_method, AuthMethod::OAuth);
    }

    #[test]
    fn new_oauth_ready_expired_overridden_by_valid() {
        let past = now() - chrono::Duration::hours(1);
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(past),
        };
        let result = AuthStatusResult::new(&resolved, Some(&valid_validation()), now());
        // The I/O layer proactively refreshes expired tokens before the
        // validation request.  If validation succeeded (Valid), the token
        // was refreshed — report Valid, not Expired.
        assert_eq!(result.status, AuthStatus::Valid);
    }

    #[test]
    fn new_oauth_ready_expired_not_overridden_by_invalid() {
        let past = now() - chrono::Duration::hours(1);
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(past),
        };
        let result = AuthStatusResult::new(&resolved, Some(&invalid_validation()), now());
        // If validation returned Invalid, the refresh failed — Expired
        // is still the most accurate status.
        assert_eq!(result.status, AuthStatus::Invalid);
    }

    #[test]
    fn new_oauth_ready_expired_not_overridden_by_not_validated() {
        let past = now() - chrono::Duration::hours(1);
        let resolved = ResolvedAuth::OAuthReady {
            expires_at: Some(past),
        };
        let result = AuthStatusResult::new(&resolved, Some(&not_validated_validation()), now());
        // No validation result — Expired status preserved.
        assert_eq!(result.status, AuthStatus::Expired);
    }

    #[test]
    fn new_not_configured_not_overridden_by_valid() {
        let resolved = ResolvedAuth::NotConfigured {
            configured_method: AuthMethod::Basic,
            detail: "No creds".into(),
        };
        let result = AuthStatusResult::new(&resolved, Some(&valid_validation()), now());
        assert_eq!(result.status, AuthStatus::NotConfigured);
    }

    #[test]
    fn new_oauth_pending_not_overridden_by_valid() {
        let result = AuthStatusResult::new(
            &ResolvedAuth::OAuthPending,
            Some(&valid_validation()),
            now(),
        );
        assert_eq!(result.status, AuthStatus::NotConfigured);
    }

    #[test]
    fn new_not_configured_not_overridden_by_invalid() {
        let resolved = ResolvedAuth::NotConfigured {
            configured_method: AuthMethod::Auto,
            detail: "Nothing".into(),
        };
        let result = AuthStatusResult::new(&resolved, Some(&invalid_validation()), now());
        assert_eq!(result.status, AuthStatus::NotConfigured);
    }

    #[test]
    fn new_matches_from_resolved_when_no_validation() {
        // Verify backward compatibility: new(..., None, ...) == from_resolved(...)
        let resolved_states = vec![
            ResolvedAuth::Basic,
            ResolvedAuth::OAuthReady { expires_at: None },
            ResolvedAuth::OAuthPending,
            ResolvedAuth::NotConfigured {
                configured_method: AuthMethod::Auto,
                detail: "test".into(),
            },
        ];
        for resolved in &resolved_states {
            let from_new = AuthStatusResult::new(resolved, None, now());
            let from_old = AuthStatusResult::from_resolved(resolved, now());
            assert_eq!(from_new, from_old, "mismatch for {resolved:?}");
        }
    }
}
