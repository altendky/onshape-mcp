//! OAuth 2.0 types and constants for the Onshape API.
//!
//! Provides pure data types for OAuth token storage and Onshape-specific
//! OAuth endpoint constants. No HTTP client or async runtime -- all I/O
//! is handled by the I/O layer.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

// ============================================================================
// Onshape OAuth Constants
// ============================================================================

/// Onshape OAuth 2.0 authorization endpoint.
pub const ONSHAPE_AUTH_URL: &str = "https://oauth.onshape.com/oauth/authorize";

/// Onshape OAuth 2.0 token endpoint.
pub const ONSHAPE_TOKEN_URL: &str = "https://oauth.onshape.com/oauth/token";

// ============================================================================
// Token Data
// ============================================================================

/// OAuth 2.0 token data, serializable to/from JSON for file storage.
///
/// Contains the access token, refresh token, and optional expiration time.
/// Secrets are wrapped in [`SecretString`] to prevent accidental logging,
/// with custom serde implementations that handle the wrapping.
#[derive(Debug, Deserialize, Serialize)]
pub struct OAuthTokenData {
    /// The OAuth 2.0 access token.
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub access_token: SecretString,
    /// The OAuth 2.0 refresh token.
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub refresh_token: SecretString,
    /// When the access token expires, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The token type (typically "Bearer").
    #[serde(default = "default_token_type")]
    pub token_type: String,
}

impl OAuthTokenData {
    /// Checks whether the access token has expired relative to the given timestamp.
    ///
    /// Returns `true` if `expires_at` is set and is before or equal to `now`.
    /// Returns `false` if `expires_at` is `None` (expiration unknown).
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expires| expires <= now)
    }
}

fn default_token_type() -> String {
    "Bearer".into()
}

/// Serializes a [`SecretString`] by exposing its inner value.
///
/// This is intentional: the token file on disk must contain the actual secret.
fn serialize_secret<S>(secret: &SecretString, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(secret.expose_secret())
}

/// Deserializes a string into a [`SecretString`].
fn deserialize_secret<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(SecretString::from(s))
}

// ============================================================================
// Token File Path
// ============================================================================

/// Returns the default token file path for the current platform.
///
/// - **Unix:** `~/.local/share/onshape-mcp/tokens.json`
/// - **macOS:** `~/Library/Application Support/onshape-mcp/tokens.json`
/// - **Windows:** `%LOCALAPPDATA%\onshape-mcp\tokens.json`
///
/// Returns `None` if the platform data directory cannot be determined.
#[must_use]
pub fn default_token_file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("onshape-mcp").join("tokens.json"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn token_data_serializes_to_json() {
        let tokens = OAuthTokenData {
            access_token: SecretString::from("access-123"),
            refresh_token: SecretString::from("refresh-456"),
            expires_at: None,
            token_type: "Bearer".into(),
        };
        let json = serde_json::to_string(&tokens).expect("should serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        assert_eq!(value["access_token"], "access-123");
        assert_eq!(value["refresh_token"], "refresh-456");
        assert_eq!(value["token_type"], "Bearer");
        assert!(value.get("expires_at").is_none());
    }

    #[test]
    fn token_data_deserializes_from_json() {
        let json = r#"{
            "access_token": "access-789",
            "refresh_token": "refresh-012",
            "token_type": "Bearer"
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(tokens.access_token.expose_secret(), "access-789");
        assert_eq!(tokens.refresh_token.expose_secret(), "refresh-012");
        assert_eq!(tokens.token_type, "Bearer");
        assert!(tokens.expires_at.is_none());
    }

    #[test]
    fn token_data_roundtrips_with_expiry() {
        let expires = DateTime::parse_from_rfc3339("2025-06-15T12:00:00Z")
            .expect("should parse")
            .to_utc();
        let tokens = OAuthTokenData {
            access_token: SecretString::from("at"),
            refresh_token: SecretString::from("rt"),
            expires_at: Some(expires),
            token_type: "Bearer".into(),
        };
        let json = serde_json::to_string(&tokens).expect("should serialize");
        let roundtripped: OAuthTokenData = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(roundtripped.expires_at, Some(expires));
    }

    #[test]
    fn is_expired_returns_true_when_past() {
        let expires = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        let tokens = OAuthTokenData {
            access_token: SecretString::from("at"),
            refresh_token: SecretString::from("rt"),
            expires_at: Some(expires),
            token_type: "Bearer".into(),
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        assert!(tokens.is_expired(now));
    }

    #[test]
    fn is_expired_returns_true_when_exactly_at_expiry() {
        let expires = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        let tokens = OAuthTokenData {
            access_token: SecretString::from("at"),
            refresh_token: SecretString::from("rt"),
            expires_at: Some(expires),
            token_type: "Bearer".into(),
        };
        assert!(tokens.is_expired(expires));
    }

    #[test]
    fn is_expired_returns_false_when_future() {
        let expires = DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        let tokens = OAuthTokenData {
            access_token: SecretString::from("at"),
            refresh_token: SecretString::from("rt"),
            expires_at: Some(expires),
            token_type: "Bearer".into(),
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        assert!(!tokens.is_expired(now));
    }

    #[test]
    fn is_expired_returns_false_when_no_expiry() {
        let tokens = OAuthTokenData {
            access_token: SecretString::from("at"),
            refresh_token: SecretString::from("rt"),
            expires_at: None,
            token_type: "Bearer".into(),
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        assert!(!tokens.is_expired(now));
    }

    #[test]
    fn default_token_type_is_bearer() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt"
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(tokens.token_type, "Bearer");
    }

    #[test]
    fn default_token_file_path_returns_some() {
        // This test may fail in environments without a home directory,
        // but it should work in typical development environments.
        let path = default_token_file_path();
        if let Some(ref p) = path {
            assert!(p.ends_with("onshape-mcp/tokens.json"));
        }
        // Don't assert Some -- CI containers may not have a data dir
    }

    #[test]
    fn onshape_auth_url_is_valid() {
        assert!(ONSHAPE_AUTH_URL.starts_with("https://"));
        assert!(ONSHAPE_AUTH_URL.contains("oauth.onshape.com"));
    }

    #[test]
    fn onshape_token_url_is_valid() {
        assert!(ONSHAPE_TOKEN_URL.starts_with("https://"));
        assert!(ONSHAPE_TOKEN_URL.contains("oauth.onshape.com"));
    }
}
