//! Authentication types and logic for the Onshape API.
//!
//! Provides pure functions for generating authorization headers from API credentials.
//! Supports Basic authentication and OAuth 2.0 bearer tokens.
//! HMAC-SHA256 request signing is planned as a future enhancement.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

// ============================================================================
// Types
// ============================================================================

/// Supported authentication methods for the Onshape API.
///
/// See the [Onshape API key docs](https://onshape-public.github.io/docs/auth/apikeys/)
/// for details on each method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthMethod {
    /// HTTP Basic authentication: base64-encoded `access_key:secret_key`.
    ///
    /// Simplest method. Relies on HTTPS for transport security.
    /// Onshape documents this as suitable for local testing and personal use.
    Basic,
    /// OAuth 2.0 bearer token authentication.
    ///
    /// Uses access tokens obtained through the OAuth 2.0 authorization code flow.
    /// Tokens are stored in a local file and refreshed when expired.
    /// Suitable for multi-user apps and team access.
    #[serde(rename = "oauth")]
    OAuth,
    // Future: HMAC-SHA256 request signing.
    // Each request is signed with a nonce and timestamp, providing replay
    // protection and avoiding sending the secret key over the wire.
    // See: docs/src/project/open-questions.md
}

/// API key credentials for authenticating with the Onshape API.
pub struct Credentials {
    /// The API access key (acts as a username/identifier).
    pub access_key: SecretString,
    /// The API secret key (acts as a password/signing key).
    pub secret_key: SecretString,
}

// ============================================================================
// Authorization Header Generation
// ============================================================================

/// Generates the value for the HTTP `Authorization` header.
///
/// The returned string is wrapped in [`SecretString`] because it contains
/// encoded credentials that should not be logged.
///
/// # Arguments
///
/// * `credentials` — The API key pair to authenticate with.
/// * `method` — Which authentication method to use.
///
/// # Examples
///
/// ```
/// use onshape_client_core::auth::{AuthMethod, Credentials, authorization_header_value};
/// use secrecy::{ExposeSecret, SecretString};
///
/// let creds = Credentials {
///     access_key: SecretString::from("my_access_key"),
///     secret_key: SecretString::from("my_secret_key"),
/// };
///
/// let header = authorization_header_value(&creds, AuthMethod::Basic);
/// assert!(header.expose_secret().starts_with("Basic "));
/// ```
#[must_use]
pub fn authorization_header_value(credentials: &Credentials, method: AuthMethod) -> SecretString {
    match method {
        AuthMethod::Basic => basic_authorization_header_value(credentials),
        AuthMethod::OAuth => {
            // OAuth uses bearer tokens, not API key credentials.
            // Callers should use `bearer_authorization_header_value()` instead.
            // This arm exists for exhaustiveness; passing API-key credentials
            // when OAuth is configured is a caller error, but we produce a
            // valid (though useless) Basic header rather than panicking.
            basic_authorization_header_value(credentials)
        }
    }
}

/// Generates a Bearer token `Authorization` header value for OAuth 2.0.
///
/// Format: `Bearer <access_token>`
///
/// The returned string is wrapped in [`SecretString`] because it contains
/// the access token that should not be logged.
///
/// # Arguments
///
/// * `access_token` — The OAuth 2.0 access token.
#[must_use]
pub fn bearer_authorization_header_value(access_token: &SecretString) -> SecretString {
    SecretString::from(format!("Bearer {}", access_token.expose_secret()))
}

/// Generates a Basic auth `Authorization` header value.
///
/// Format: `Basic <base64(access_key:secret_key)>`
fn basic_authorization_header_value(credentials: &Credentials) -> SecretString {
    let access = credentials.access_key.expose_secret();
    let secret = credentials.secret_key.expose_secret();
    let encoded = BASE64.encode(format!("{access}:{secret}"));
    SecretString::from(format!("Basic {encoded}"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn test_credentials() -> Credentials {
        Credentials {
            access_key: SecretString::from("my_access_key"),
            secret_key: SecretString::from("my_secret_key"),
        }
    }

    #[test]
    fn basic_auth_starts_with_basic_prefix() {
        let creds = test_credentials();
        let header = authorization_header_value(&creds, AuthMethod::Basic);
        assert!(header.expose_secret().starts_with("Basic "));
    }

    #[test]
    fn basic_auth_encodes_correctly() {
        let creds = test_credentials();
        let header = authorization_header_value(&creds, AuthMethod::Basic);
        let value = header.expose_secret();

        // Strip "Basic " prefix and decode
        let encoded = value
            .strip_prefix("Basic ")
            .expect("should have Basic prefix");
        let decoded_bytes = BASE64.decode(encoded).expect("should be valid base64");
        let decoded = String::from_utf8(decoded_bytes).expect("should be valid UTF-8");

        assert_eq!(decoded, "my_access_key:my_secret_key");
    }

    #[test]
    fn basic_auth_matches_known_value() {
        // Verify against a value computed independently:
        // echo -n "access:secret" | base64 => "YWNjZXNzOnNlY3JldA=="
        let creds = Credentials {
            access_key: SecretString::from("access"),
            secret_key: SecretString::from("secret"),
        };
        let header = authorization_header_value(&creds, AuthMethod::Basic);
        assert_eq!(header.expose_secret(), "Basic YWNjZXNzOnNlY3JldA==");
    }

    #[test]
    fn basic_auth_handles_empty_keys() {
        let creds = Credentials {
            access_key: SecretString::from(""),
            secret_key: SecretString::from(""),
        };
        let header = authorization_header_value(&creds, AuthMethod::Basic);
        let value = header.expose_secret();

        let encoded = value
            .strip_prefix("Basic ")
            .expect("should have Basic prefix");
        let decoded_bytes = BASE64.decode(encoded).expect("should be valid base64");
        let decoded = String::from_utf8(decoded_bytes).expect("should be valid UTF-8");

        assert_eq!(decoded, ":");
    }

    #[test]
    fn basic_auth_handles_special_characters() {
        let creds = Credentials {
            access_key: SecretString::from("key+with/special=chars"),
            secret_key: SecretString::from("s3cr3t!@#$%^&*()"),
        };
        let header = authorization_header_value(&creds, AuthMethod::Basic);
        let value = header.expose_secret();

        let encoded = value
            .strip_prefix("Basic ")
            .expect("should have Basic prefix");
        let decoded_bytes = BASE64.decode(encoded).expect("should be valid base64");
        let decoded = String::from_utf8(decoded_bytes).expect("should be valid UTF-8");

        assert_eq!(decoded, "key+with/special=chars:s3cr3t!@#$%^&*()");
    }

    #[test]
    fn basic_auth_handles_colon_in_keys() {
        // Colons in keys are allowed — the first colon separates access from secret
        // when decoding, but for encoding it's just part of the concatenated string.
        let creds = Credentials {
            access_key: SecretString::from("key:with:colons"),
            secret_key: SecretString::from("secret:too"),
        };
        let header = authorization_header_value(&creds, AuthMethod::Basic);
        let value = header.expose_secret();

        let encoded = value
            .strip_prefix("Basic ")
            .expect("should have Basic prefix");
        let decoded_bytes = BASE64.decode(encoded).expect("should be valid base64");
        let decoded = String::from_utf8(decoded_bytes).expect("should be valid UTF-8");

        assert_eq!(decoded, "key:with:colons:secret:too");
    }

    #[test]
    fn auth_method_serializes_to_snake_case() {
        let json = serde_json::to_string(&AuthMethod::Basic).expect("should serialize");
        assert_eq!(json, "\"basic\"");
    }

    #[test]
    fn auth_method_deserializes_from_snake_case() {
        let method: AuthMethod = serde_json::from_str("\"basic\"").expect("should deserialize");
        assert_eq!(method, AuthMethod::Basic);
    }

    #[test]
    fn auth_method_oauth_serializes_to_snake_case() {
        let json = serde_json::to_string(&AuthMethod::OAuth).expect("should serialize");
        assert_eq!(json, "\"oauth\"");
    }

    #[test]
    fn auth_method_oauth_deserializes_from_snake_case() {
        let method: AuthMethod = serde_json::from_str("\"oauth\"").expect("should deserialize");
        assert_eq!(method, AuthMethod::OAuth);
    }

    #[test]
    fn bearer_auth_starts_with_bearer_prefix() {
        let token = SecretString::from("test-access-token");
        let header = bearer_authorization_header_value(&token);
        assert!(header.expose_secret().starts_with("Bearer "));
    }

    #[test]
    fn bearer_auth_contains_token() {
        let token = SecretString::from("my-oauth-token-12345");
        let header = bearer_authorization_header_value(&token);
        assert_eq!(header.expose_secret(), "Bearer my-oauth-token-12345");
    }

    #[test]
    fn bearer_auth_handles_empty_token() {
        let token = SecretString::from("");
        let header = bearer_authorization_header_value(&token);
        assert_eq!(header.expose_secret(), "Bearer ");
    }
}
