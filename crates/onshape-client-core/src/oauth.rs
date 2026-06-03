//! OAuth 2.0 types and constants for the Onshape API.
//!
//! Provides pure data types for OAuth token material, Onshape-specific
//! OAuth endpoint constants, and an [`oauth2`] client builder.
//! No HTTP client or async runtime — all I/O is handled by the I/O layer.

use chrono::{DateTime, Utc};
use oauth2::basic::{BasicClient, BasicTokenResponse};
use oauth2::{
    AccessToken, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, RefreshToken, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// Onshape OAuth Constants
// ============================================================================

/// Onshape OAuth 2.0 authorization endpoint (string form).
const ONSHAPE_AUTH_URL_STR: &str = "https://oauth.onshape.com/oauth/authorize";

/// Onshape OAuth 2.0 token endpoint (string form).
const ONSHAPE_TOKEN_URL_STR: &str = "https://oauth.onshape.com/oauth/token";

/// Returns the Onshape OAuth 2.0 authorization endpoint as a typed [`AuthUrl`].
///
/// # Panics
///
/// Panics if the hard-coded URL cannot be parsed. This is a compile-time
/// constant so the panic is unreachable in practice.
#[must_use]
pub fn onshape_auth_url() -> AuthUrl {
    #[allow(clippy::expect_used)]
    AuthUrl::new(ONSHAPE_AUTH_URL_STR.to_string()).expect("hard-coded Onshape auth URL is valid")
}

/// Returns the Onshape OAuth 2.0 token endpoint as a typed [`TokenUrl`].
///
/// # Panics
///
/// Panics if the hard-coded URL cannot be parsed. This is a compile-time
/// constant so the panic is unreachable in practice.
#[must_use]
pub fn onshape_token_url() -> TokenUrl {
    #[allow(clippy::expect_used)]
    TokenUrl::new(ONSHAPE_TOKEN_URL_STR.to_string()).expect("hard-coded Onshape token URL is valid")
}

// ============================================================================
// Token Data
// ============================================================================

/// OAuth 2.0 token material.
///
/// Contains the access token, refresh token, and optional expiration time.
/// Token values use [`oauth2::AccessToken`] and [`oauth2::RefreshToken`] types,
/// with custom serde implementations for JSON persistence by callers.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthTokenData {
    /// The OAuth 2.0 access token.
    #[serde(
        serialize_with = "serialize_access_token",
        deserialize_with = "deserialize_access_token"
    )]
    pub access_token: AccessToken,
    /// The OAuth 2.0 refresh token.
    #[serde(
        serialize_with = "serialize_refresh_token",
        deserialize_with = "deserialize_refresh_token"
    )]
    pub refresh_token: RefreshToken,
    /// When the access token expires, if known.
    /// Stored as an absolute timestamp for persistence (unlike the relative
    /// `expires_in` from the token response).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The token type — must be "bearer" (case-insensitive).
    ///
    /// Validated during deserialization: rejects non-bearer token types to
    /// catch corrupted or tampered persisted token data early. The value is
    /// normalized to lowercase on load.
    #[serde(
        default = "default_token_type",
        deserialize_with = "deserialize_token_type"
    )]
    pub token_type: String,
    /// OAuth 2.0 scopes granted by the authorization server.
    ///
    /// Stored as a list of scope strings (e.g. `["OAuth2Read", "OAuth2Write"]`).
    /// `None` when the server did not return scopes or the token predates
    /// scope tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
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

    /// Returns `true` if the token expires within `margin` of `now`, or is already expired.
    ///
    /// Returns `false` if `expires_at` is `None` (unknown expiry — assume valid).
    #[must_use]
    pub fn is_expiring_soon(&self, now: DateTime<Utc>, margin: chrono::Duration) -> bool {
        self.expires_at
            .is_some_and(|expires| expires <= now + margin)
    }
}

impl OAuthTokenData {
    /// Converts an [`oauth2::basic::BasicTokenResponse`] into [`OAuthTokenData`].
    ///
    /// The relative `expires_in` duration from the token response is converted
    /// to an absolute `expires_at` timestamp using the provided `now` value.
    /// Accepting `now` as a parameter (instead of calling [`Utc::now()`])
    /// keeps this function pure and testable with exact timestamps.
    #[must_use]
    pub fn from_response(response: &BasicTokenResponse, now: DateTime<Utc>) -> Self {
        let expires_at = response
            .expires_in()
            .and_then(|d| chrono::Duration::from_std(d).ok())
            .map(|d| now + d);

        let scopes = response
            .scopes()
            .map(|scopes| scopes.iter().map(|s| s.as_ref().to_owned()).collect());

        Self {
            access_token: response.access_token().clone(),
            refresh_token: response
                .refresh_token()
                .cloned()
                .unwrap_or_else(|| RefreshToken::new(String::new())),
            expires_at,
            token_type: response.token_type().as_ref().to_string(),
            scopes,
        }
    }
}

impl OAuthTokenData {
    /// Build token data from raw field values (e.g. parsed from a proxy response).
    ///
    #[must_use]
    pub fn from_raw(
        access_token: String,
        refresh_token: String,
        expires_at: Option<DateTime<Utc>>,
        token_type: String,
        scopes: Option<Vec<String>>,
    ) -> Self {
        Self {
            access_token: AccessToken::new(access_token),
            refresh_token: RefreshToken::new(refresh_token),
            expires_at,
            token_type,
            scopes,
        }
    }
}

fn default_token_type() -> String {
    "bearer".into()
}

// ============================================================================
// Serde Helpers for oauth2 types
// ============================================================================

/// Deserializes and validates the `token_type` field.
///
/// Accepts "bearer" (case-insensitive) and normalizes to lowercase.
/// Rejects any other token type to catch corrupted or tampered persisted token data.
fn deserialize_token_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.eq_ignore_ascii_case("bearer") {
        Ok("bearer".to_string())
    } else {
        Err(serde::de::Error::custom(format!(
            "invalid token_type \"{s}\", expected \"bearer\""
        )))
    }
}

/// Serializes an [`AccessToken`] by exposing its secret value.
///
/// This is intentional: serialized token material must contain the actual secret.
fn serialize_access_token<S>(token: &AccessToken, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(token.secret())
}

/// Deserializes a string into an [`AccessToken`].
fn deserialize_access_token<'de, D>(deserializer: D) -> Result<AccessToken, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(AccessToken::new(s))
}

/// Serializes a [`RefreshToken`] by exposing its secret value.
fn serialize_refresh_token<S>(token: &RefreshToken, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(token.secret())
}

/// Deserializes a string into a [`RefreshToken`].
fn deserialize_refresh_token<'de, D>(deserializer: D) -> Result<RefreshToken, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(RefreshToken::new(s))
}

// ============================================================================
// Client Builder
// ============================================================================

/// A [`BasicClient`] configured with Onshape's auth and token endpoints.
///
/// The type parameters encode that the authorization URL and token URL are set,
/// while the device-auth, introspection, and revocation endpoints are not.
pub type OnshapeOAuthClient = BasicClient<
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

/// Creates a configured [`OnshapeOAuthClient`] for Onshape OAuth 2.0.
///
/// Sets the authorization and token endpoints to the Onshape URLs.
/// The returned client is ready for authorization code exchanges and
/// token refresh operations — but performs no I/O itself.
///
/// # Arguments
///
/// * `client_id` — The OAuth 2.0 client ID from Onshape.
/// * `client_secret` — The OAuth 2.0 client secret from Onshape.
#[must_use]
pub fn onshape_oauth_client(client_id: &str, client_secret: &str) -> OnshapeOAuthClient {
    BasicClient::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(client_secret.to_string()))
        .set_auth_uri(onshape_auth_url())
        .set_token_uri(onshape_token_url())
}

// ============================================================================
// OAuth Session (Refresh State Machine)
// ============================================================================

/// Action a caller should take *before* executing an API request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreExecuteAction {
    /// Token is valid — proceed with the current access token.
    Proceed,
    /// Token is expiring soon or already expired — attempt refresh first.
    RefreshNeeded,
}

/// Action a caller should take *after* receiving an API response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostExecuteAction {
    /// Response is usable — return it to the caller.
    Done,
    /// Got 401 and haven't refreshed yet — refresh and retry once.
    RefreshAndRetry,
}

/// Manages OAuth token lifecycle decisions. Pure computation — no I/O.
///
/// Callers can own an `OAuthSession` and consult it before and after each API
/// request to decide whether a token refresh is needed.
pub struct OAuthSession {
    /// Current token data. Public so callers can persist or inspect it.
    pub tokens: OAuthTokenData,
    refresh_margin: chrono::Duration,
}

impl OAuthSession {
    /// Creates a new session with the given tokens and refresh margin.
    ///
    /// The `refresh_margin` is how far before expiry the proactive refresh
    /// should trigger (e.g. 60 seconds).
    #[must_use]
    pub const fn new(tokens: OAuthTokenData, refresh_margin: chrono::Duration) -> Self {
        Self {
            tokens,
            refresh_margin,
        }
    }

    /// Decide whether to refresh before making a request.
    ///
    /// Injects `now` for testability.
    #[must_use]
    pub fn pre_execute_action(&self, now: DateTime<Utc>) -> PreExecuteAction {
        if self.tokens.is_expiring_soon(now, self.refresh_margin) {
            PreExecuteAction::RefreshNeeded
        } else {
            PreExecuteAction::Proceed
        }
    }

    /// Decide what to do after an API response.
    ///
    /// `already_refreshed` prevents infinite refresh loops: if we already
    /// refreshed once during this request cycle and still got 401, give up.
    #[must_use]
    pub const fn post_execute_action(
        &self,
        status: u16,
        already_refreshed: bool,
    ) -> PostExecuteAction {
        if status == 401 && !already_refreshed {
            PostExecuteAction::RefreshAndRetry
        } else {
            PostExecuteAction::Done
        }
    }

    /// Apply a successful refresh response.
    ///
    /// Converts the `expires_in` duration to an absolute `expires_at`
    /// timestamp using the provided `now` value. The caller is responsible
    /// for persisting the updated tokens and rebuilding any HTTP client.
    pub fn apply_refresh(&mut self, response: &BasicTokenResponse, now: DateTime<Utc>) {
        let mut new_tokens = OAuthTokenData::from_response(response, now);
        // Per RFC 6749 Section 6: if the server omits refresh_token in the
        // response, the client must keep the existing one.
        if response.refresh_token().is_none() {
            new_tokens.refresh_token = self.tokens.refresh_token.clone();
        }
        self.tokens = new_tokens;
    }

    /// Try adopting externally-refreshed tokens, such as tokens refreshed by
    /// another process.
    ///
    /// Returns `true` if the external tokens were fresher and were adopted.
    /// Returns `false` (tokens unchanged) if:
    /// - The external tokens have the same or earlier expiry
    /// - Either side has no expiry set (`None`)
    /// - The external tokens are already expired
    pub fn apply_external_tokens(
        &mut self,
        external_tokens: OAuthTokenData,
        now: DateTime<Utc>,
    ) -> bool {
        // Both must have a known expiry to compare.
        let (Some(external_expires), Some(current_expires)) =
            (external_tokens.expires_at, self.tokens.expires_at)
        else {
            return false;
        };

        // External tokens must be fresher and not already expired.
        if external_expires > current_expires && external_expires > now {
            self.tokens = external_tokens;
            true
        } else {
            false
        }
    }

    /// Returns a reference to the current access token.
    #[must_use]
    pub const fn access_token(&self) -> &AccessToken {
        &self.tokens.access_token
    }

    /// Returns a reference to the current refresh token.
    #[must_use]
    pub const fn refresh_token(&self) -> &RefreshToken {
        &self.tokens.refresh_token
    }
}

// ============================================================================
// OAuth Login Flow (Authorization Code + PKCE)
// ============================================================================

/// Configuration for starting an OAuth login flow.
///
/// Contains the OAuth client ID, redirect URI, and requested scopes.
/// Used to build the authorization URL and validate the callback.
#[derive(Clone, Debug)]
pub struct OAuthLoginConfig {
    /// The OAuth 2.0 client ID.
    pub client_id: String,
    /// The redirect URI where Onshape will send the authorization code.
    /// Typically `http://127.0.0.1:18338/callback`.
    pub redirect_uri: String,
    /// OAuth 2.0 scopes to request (e.g. `["OAuth2Read", "OAuth2Write"]`).
    pub scopes: Vec<String>,
}

/// Active OAuth login session state.
///
/// Holds the PKCE verifier and CSRF state needed to complete the
/// authorization code exchange after the user authorizes in their browser.
/// This is a pure state machine — no I/O.
pub struct OAuthLoginSession {
    /// PKCE code verifier — kept secret until the token exchange.
    pub pkce_verifier: PkceCodeVerifier,
    /// CSRF state token — validated against the callback to prevent CSRF.
    pub csrf_state: CsrfToken,
    /// The login configuration used to start this session.
    pub config: OAuthLoginConfig,
}

/// Errors that can occur during OAuth callback validation.
#[derive(Debug, thiserror::Error)]
pub enum CallbackValidationError {
    /// The callback URL could not be parsed.
    #[error("invalid callback URL: {0}")]
    InvalidUrl(String),
    /// The OAuth provider returned an error.
    #[error("OAuth error from provider: {error} (description: {description:?})")]
    OAuthError {
        /// The OAuth error code (e.g. `access_denied`).
        error: String,
        /// Optional human-readable error description.
        description: Option<String>,
    },
    /// The CSRF state token does not match.
    #[error("CSRF state mismatch: expected {expected}, got {actual}")]
    StateMismatch {
        /// The expected state token.
        expected: String,
        /// The actual state token from the callback.
        actual: String,
    },
    /// The callback did not contain a `state` parameter.
    #[error("callback is missing the 'state' parameter")]
    MissingState,
    /// The callback did not contain a `code` parameter.
    #[error("callback is missing the 'code' parameter")]
    MissingCode,
}

/// Constructs the Onshape OAuth authorization URL.
///
/// Builds a full authorization URL with PKCE challenge, CSRF state,
/// redirect URI, and requested scopes. The returned URL should be
/// opened in the user's browser.
///
/// # Arguments
///
/// * `config` — OAuth login configuration (client ID, redirect URI, scopes)
/// * `csrf_state` — CSRF state token to include in the request
/// * `pkce_challenge` — PKCE code challenge (S256)
///
/// This is a pure function — no I/O.
///
/// # Panics
///
/// Panics if the `redirect_uri` in `config` is not a valid URL.
#[must_use]
pub fn build_authorize_url(
    config: &OAuthLoginConfig,
    csrf_state: &CsrfToken,
    pkce_challenge: PkceCodeChallenge,
) -> String {
    let client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(onshape_auth_url())
        .set_token_uri(onshape_token_url())
        .set_redirect_uri(
            #[allow(clippy::expect_used)]
            RedirectUrl::new(config.redirect_uri.clone())
                .expect("redirect_uri should be a valid URL"),
        );

    let mut auth_request = client
        .authorize_url(|| csrf_state.clone())
        .set_pkce_challenge(pkce_challenge);

    for scope in &config.scopes {
        auth_request = auth_request.add_scope(oauth2::Scope::new(scope.clone()));
    }

    let (url, _csrf_token) = auth_request.url();
    url.to_string()
}

/// Validates an OAuth callback URL and extracts the authorization code.
///
/// Checks for OAuth error parameters, validates the CSRF state token,
/// and extracts the authorization code from the callback URL.
///
/// # Arguments
///
/// * `callback_url` — The full callback URL received from Onshape
/// * `expected_state` — The CSRF state token from the login session
///
/// # Errors
///
/// Returns an error if:
/// - The URL cannot be parsed
/// - The OAuth provider returned an error
/// - The CSRF state does not match
/// - The authorization code is missing
pub fn validate_callback(
    callback_url: &str,
    expected_state: &CsrfToken,
) -> Result<AuthorizationCode, CallbackValidationError> {
    let url = url::Url::parse(callback_url)
        .map_err(|e| CallbackValidationError::InvalidUrl(e.to_string()))?;

    let params: std::collections::HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Check for OAuth error response.
    if let Some(error) = params.get("error") {
        return Err(CallbackValidationError::OAuthError {
            error: error.clone(),
            description: params.get("error_description").cloned(),
        });
    }

    // Validate CSRF state.
    let state = params
        .get("state")
        .ok_or(CallbackValidationError::MissingState)?;

    if state != expected_state.secret() {
        return Err(CallbackValidationError::StateMismatch {
            expected: expected_state.secret().clone(),
            actual: state.clone(),
        });
    }

    // Extract authorization code.
    let code = params
        .get("code")
        .ok_or(CallbackValidationError::MissingCode)?;

    Ok(AuthorizationCode::new(code.clone()))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn token_data_serializes_to_json() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("access-123".to_string()),
            refresh_token: RefreshToken::new("refresh-456".to_string()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        };
        let json = serde_json::to_string(&tokens).expect("should serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        assert_eq!(value["access_token"], "access-123");
        assert_eq!(value["refresh_token"], "refresh-456");
        assert_eq!(value["token_type"], "bearer");
        assert!(value.get("expires_at").is_none());
        assert!(value.get("scopes").is_none());
    }

    #[test]
    fn token_data_deserializes_from_json() {
        let json = r#"{
            "access_token": "access-789",
            "refresh_token": "refresh-012",
            "token_type": "bearer"
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(tokens.access_token.secret(), "access-789");
        assert_eq!(tokens.refresh_token.secret(), "refresh-012");
        assert_eq!(tokens.token_type, "bearer");
        assert!(tokens.expires_at.is_none());
        assert!(tokens.scopes.is_none());
    }

    #[test]
    fn token_data_roundtrips_with_expiry() {
        let expires = DateTime::parse_from_rfc3339("2025-06-15T12:00:00Z")
            .expect("should parse")
            .to_utc();
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: Some(expires),
            token_type: "bearer".into(),
            scopes: None,
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
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: Some(expires),
            token_type: "bearer".into(),
            scopes: None,
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
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: Some(expires),
            token_type: "bearer".into(),
            scopes: None,
        };
        assert!(tokens.is_expired(expires));
    }

    #[test]
    fn is_expired_returns_false_when_future() {
        let expires = DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: Some(expires),
            token_type: "bearer".into(),
            scopes: None,
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("should parse")
            .to_utc();
        assert!(!tokens.is_expired(now));
    }

    #[test]
    fn is_expired_returns_false_when_no_expiry() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
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
        assert_eq!(tokens.token_type, "bearer");
    }

    #[test]
    fn token_type_bearer_case_insensitive() {
        // "Bearer" (capitalized) should be accepted and normalized to lowercase.
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "token_type": "Bearer"
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(tokens.token_type, "bearer");
    }

    #[test]
    fn token_type_bearer_all_caps() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "token_type": "BEARER"
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(tokens.token_type, "bearer");
    }

    #[test]
    fn token_type_invalid_rejects() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "token_type": "mac"
        }"#;
        let result: Result<OAuthTokenData, _> = serde_json::from_str(json);
        let err = result.expect_err("should reject non-bearer token type");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid token_type"),
            "error should mention invalid token_type: {msg}"
        );
    }

    #[test]
    fn scopes_deserialize_when_present() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "token_type": "bearer",
            "scopes": ["OAuth2Read", "OAuth2Write"]
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        let scopes = tokens.scopes.expect("should have scopes");
        assert_eq!(scopes, vec!["OAuth2Read", "OAuth2Write"]);
    }

    #[test]
    fn scopes_default_to_none_when_absent() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "token_type": "bearer"
        }"#;
        let tokens: OAuthTokenData = serde_json::from_str(json).expect("should deserialize");
        assert!(tokens.scopes.is_none());
    }

    #[test]
    fn scopes_serialize_when_present() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: Some(vec!["OAuth2Read".into(), "OAuth2Write".into()]),
        };
        let json = serde_json::to_string(&tokens).expect("should serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        let scopes = value["scopes"].as_array().expect("scopes should be array");
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0], "OAuth2Read");
        assert_eq!(scopes[1], "OAuth2Write");
    }

    #[test]
    fn scopes_omitted_from_json_when_none() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".to_string()),
            refresh_token: RefreshToken::new("rt".to_string()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        };
        let json = serde_json::to_string(&tokens).expect("should serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        assert!(
            value.get("scopes").is_none(),
            "scopes should be omitted from JSON when None"
        );
    }

    #[test]
    fn onshape_auth_url_is_valid() {
        let url = onshape_auth_url();
        let url_str = url.url().as_str();
        assert!(url_str.starts_with("https://"));
        assert!(url_str.contains("oauth.onshape.com"));
    }

    #[test]
    fn onshape_token_url_is_valid() {
        let url = onshape_token_url();
        let url_str = url.url().as_str();
        assert!(url_str.starts_with("https://"));
        assert!(url_str.contains("oauth.onshape.com"));
    }

    #[test]
    fn onshape_oauth_client_builds_successfully() {
        let _client = onshape_oauth_client("test-client-id", "test-client-secret");
    }

    #[test]
    fn from_response_with_expiry() {
        let json = r#"{
            "access_token": "test-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "test-refresh-token"
        }"#;
        let response: BasicTokenResponse =
            serde_json::from_str(json).expect("should deserialize token response");
        let now = DateTime::parse_from_rfc3339("2025-06-01T12:00:00Z")
            .expect("parse")
            .to_utc();

        let token_data = OAuthTokenData::from_response(&response, now);

        assert_eq!(token_data.access_token.secret(), "test-access-token");
        assert_eq!(token_data.refresh_token.secret(), "test-refresh-token");

        let expires_at = token_data.expires_at.expect("should have expiry");
        assert_eq!(
            expires_at,
            now + chrono::Duration::seconds(3600),
            "expires_at should be exactly now + 3600s"
        );
    }

    #[test]
    fn from_response_without_expiry() {
        let json = r#"{
            "access_token": "test-access-token",
            "token_type": "Bearer"
        }"#;
        let response: BasicTokenResponse =
            serde_json::from_str(json).expect("should deserialize token response");
        let now = DateTime::parse_from_rfc3339("2025-06-01T12:00:00Z")
            .expect("parse")
            .to_utc();

        let token_data = OAuthTokenData::from_response(&response, now);

        assert_eq!(token_data.access_token.secret(), "test-access-token");
        assert!(token_data.expires_at.is_none());
        // No refresh token in the response → empty string fallback
        assert!(token_data.refresh_token.secret().is_empty());
        // No scopes in the response → None
        assert!(token_data.scopes.is_none());
    }

    #[test]
    fn from_response_preserves_scopes() {
        let json = r#"{
            "access_token": "test-at",
            "token_type": "Bearer",
            "refresh_token": "test-rt",
            "scope": "OAuth2Read OAuth2Write"
        }"#;
        let response: BasicTokenResponse =
            serde_json::from_str(json).expect("should deserialize token response");
        let now = DateTime::parse_from_rfc3339("2025-06-01T12:00:00Z")
            .expect("parse")
            .to_utc();

        let token_data = OAuthTokenData::from_response(&response, now);

        let scopes = token_data.scopes.expect("should have scopes");
        assert_eq!(scopes, vec!["OAuth2Read", "OAuth2Write"]);
    }

    #[test]
    fn token_data_json_shape_backward_compatible() {
        // Verify that the JSON shape produced by the new types matches
        // what the old SecretString-based types produced.
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("my-access".to_string()),
            refresh_token: RefreshToken::new("my-refresh".to_string()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        };
        let json = serde_json::to_string_pretty(&tokens).expect("should serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");

        // The JSON shape should have plain string values, not nested objects.
        assert!(value["access_token"].is_string());
        assert!(value["refresh_token"].is_string());
        assert!(value["token_type"].is_string());
        assert_eq!(value["access_token"], "my-access");
        assert_eq!(value["refresh_token"], "my-refresh");
    }

    // ====================================================================
    // is_expiring_soon tests
    // ====================================================================

    #[test]
    fn is_expiring_soon_false_when_well_before_margin() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".into()),
            refresh_token: RefreshToken::new("rt".into()),
            expires_at: Some(
                DateTime::parse_from_rfc3339("2025-01-01T00:02:00Z")
                    .expect("parse")
                    .to_utc(),
            ),
            token_type: "bearer".into(),
            scopes: None,
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        // 120s until expiry, 60s margin → not expiring soon
        assert!(!tokens.is_expiring_soon(now, chrono::Duration::seconds(60)));
    }

    #[test]
    fn is_expiring_soon_true_when_within_margin() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".into()),
            refresh_token: RefreshToken::new("rt".into()),
            expires_at: Some(
                DateTime::parse_from_rfc3339("2025-01-01T00:00:55Z")
                    .expect("parse")
                    .to_utc(),
            ),
            token_type: "bearer".into(),
            scopes: None,
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        // 55s until expiry, 60s margin → expiring soon
        assert!(tokens.is_expiring_soon(now, chrono::Duration::seconds(60)));
    }

    #[test]
    fn is_expiring_soon_true_when_already_expired() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".into()),
            refresh_token: RefreshToken::new("rt".into()),
            expires_at: Some(
                DateTime::parse_from_rfc3339("2024-12-31T23:59:00Z")
                    .expect("parse")
                    .to_utc(),
            ),
            token_type: "bearer".into(),
            scopes: None,
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        assert!(tokens.is_expiring_soon(now, chrono::Duration::seconds(60)));
    }

    #[test]
    fn is_expiring_soon_false_when_no_expiry() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".into()),
            refresh_token: RefreshToken::new("rt".into()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        };
        let now = Utc::now();
        assert!(!tokens.is_expiring_soon(now, chrono::Duration::seconds(60)));
    }

    #[test]
    fn is_expiring_soon_true_at_exact_margin_boundary() {
        let tokens = OAuthTokenData {
            access_token: AccessToken::new("at".into()),
            refresh_token: RefreshToken::new("rt".into()),
            expires_at: Some(
                DateTime::parse_from_rfc3339("2025-01-01T00:01:00Z")
                    .expect("parse")
                    .to_utc(),
            ),
            token_type: "bearer".into(),
            scopes: None,
        };
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        // Exactly 60s until expiry, 60s margin → at boundary, expires <= now+margin
        assert!(tokens.is_expiring_soon(now, chrono::Duration::seconds(60)));
    }

    // ====================================================================
    // OAuthSession: pre_execute_action tests
    // ====================================================================

    fn make_session(expires_at: Option<DateTime<Utc>>) -> OAuthSession {
        OAuthSession::new(
            OAuthTokenData {
                access_token: AccessToken::new("at".into()),
                refresh_token: RefreshToken::new("rt".into()),
                expires_at,
                token_type: "bearer".into(),
                scopes: None,
            },
            chrono::Duration::seconds(60),
        )
    }

    #[test]
    fn pre_execute_proceed_when_well_before_expiry() {
        let session = make_session(Some(
            DateTime::parse_from_rfc3339("2025-01-01T00:02:00Z")
                .expect("parse")
                .to_utc(),
        ));
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        assert_eq!(session.pre_execute_action(now), PreExecuteAction::Proceed);
    }

    #[test]
    fn pre_execute_refresh_when_within_margin() {
        let session = make_session(Some(
            DateTime::parse_from_rfc3339("2025-01-01T00:00:55Z")
                .expect("parse")
                .to_utc(),
        ));
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        assert_eq!(
            session.pre_execute_action(now),
            PreExecuteAction::RefreshNeeded
        );
    }

    #[test]
    fn pre_execute_refresh_when_already_expired() {
        let session = make_session(Some(
            DateTime::parse_from_rfc3339("2024-12-31T23:00:00Z")
                .expect("parse")
                .to_utc(),
        ));
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        assert_eq!(
            session.pre_execute_action(now),
            PreExecuteAction::RefreshNeeded
        );
    }

    #[test]
    fn pre_execute_proceed_when_no_expiry() {
        let session = make_session(None);
        let now = Utc::now();
        assert_eq!(session.pre_execute_action(now), PreExecuteAction::Proceed);
    }

    // ====================================================================
    // OAuthSession: post_execute_action tests
    // ====================================================================

    #[test]
    fn post_execute_done_on_200() {
        let session = make_session(None);
        assert_eq!(
            session.post_execute_action(200, false),
            PostExecuteAction::Done
        );
    }

    #[test]
    fn post_execute_refresh_and_retry_on_401_not_refreshed() {
        let session = make_session(None);
        assert_eq!(
            session.post_execute_action(401, false),
            PostExecuteAction::RefreshAndRetry
        );
    }

    #[test]
    fn post_execute_done_on_401_already_refreshed() {
        let session = make_session(None);
        assert_eq!(
            session.post_execute_action(401, true),
            PostExecuteAction::Done
        );
    }

    #[test]
    fn post_execute_done_on_403() {
        let session = make_session(None);
        assert_eq!(
            session.post_execute_action(403, false),
            PostExecuteAction::Done
        );
    }

    // ====================================================================
    // OAuthSession: apply_external_tokens tests
    // ====================================================================

    #[test]
    fn apply_external_tokens_adopts_fresher_tokens() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        let mut session = OAuthSession::new(
            OAuthTokenData {
                access_token: AccessToken::new("old-at".into()),
                refresh_token: RefreshToken::new("old-rt".into()),
                expires_at: Some(now + chrono::Duration::seconds(100)),
                token_type: "bearer".into(),
                scopes: None,
            },
            chrono::Duration::seconds(60),
        );
        let external_tokens = OAuthTokenData {
            access_token: AccessToken::new("new-at".into()),
            refresh_token: RefreshToken::new("new-rt".into()),
            expires_at: Some(now + chrono::Duration::seconds(3600)),
            token_type: "bearer".into(),
            scopes: None,
        };
        assert!(session.apply_external_tokens(external_tokens, now));
        assert_eq!(session.access_token().secret(), "new-at");
        assert_eq!(session.refresh_token().secret(), "new-rt");
    }

    #[test]
    fn apply_external_tokens_rejects_same_or_earlier_expiry() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        let mut session = OAuthSession::new(
            OAuthTokenData {
                access_token: AccessToken::new("current-at".into()),
                refresh_token: RefreshToken::new("current-rt".into()),
                expires_at: Some(now + chrono::Duration::seconds(3600)),
                token_type: "bearer".into(),
                scopes: None,
            },
            chrono::Duration::seconds(60),
        );
        let external_tokens = OAuthTokenData {
            access_token: AccessToken::new("external-at".into()),
            refresh_token: RefreshToken::new("external-rt".into()),
            expires_at: Some(now + chrono::Duration::seconds(3600)),
            token_type: "bearer".into(),
            scopes: None,
        };
        assert!(!session.apply_external_tokens(external_tokens, now));
        assert_eq!(session.access_token().secret(), "current-at");
    }

    #[test]
    fn apply_external_tokens_rejects_expired_external_tokens() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        let mut session = OAuthSession::new(
            OAuthTokenData {
                access_token: AccessToken::new("current-at".into()),
                refresh_token: RefreshToken::new("current-rt".into()),
                expires_at: Some(now - chrono::Duration::seconds(100)),
                token_type: "bearer".into(),
                scopes: None,
            },
            chrono::Duration::seconds(60),
        );
        let external_tokens = OAuthTokenData {
            access_token: AccessToken::new("external-at".into()),
            refresh_token: RefreshToken::new("external-rt".into()),
            expires_at: Some(now - chrono::Duration::seconds(50)),
            token_type: "bearer".into(),
            scopes: None,
        };
        assert!(!session.apply_external_tokens(external_tokens, now));
        assert_eq!(session.access_token().secret(), "current-at");
    }

    #[test]
    fn apply_external_tokens_rejects_when_both_none_expiry() {
        let now = Utc::now();
        let mut session = OAuthSession::new(
            OAuthTokenData {
                access_token: AccessToken::new("current-at".into()),
                refresh_token: RefreshToken::new("current-rt".into()),
                expires_at: None,
                token_type: "bearer".into(),
                scopes: None,
            },
            chrono::Duration::seconds(60),
        );
        let external_tokens = OAuthTokenData {
            access_token: AccessToken::new("external-at".into()),
            refresh_token: RefreshToken::new("external-rt".into()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        };
        assert!(!session.apply_external_tokens(external_tokens, now));
        assert_eq!(session.access_token().secret(), "current-at");
    }

    #[test]
    fn apply_external_tokens_rejects_when_external_has_none_expiry() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .expect("parse")
            .to_utc();
        let mut session = OAuthSession::new(
            OAuthTokenData {
                access_token: AccessToken::new("current-at".into()),
                refresh_token: RefreshToken::new("current-rt".into()),
                expires_at: Some(now + chrono::Duration::seconds(100)),
                token_type: "bearer".into(),
                scopes: None,
            },
            chrono::Duration::seconds(60),
        );
        let external_tokens = OAuthTokenData {
            access_token: AccessToken::new("external-at".into()),
            refresh_token: RefreshToken::new("external-rt".into()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        };
        assert!(!session.apply_external_tokens(external_tokens, now));
        assert_eq!(session.access_token().secret(), "current-at");
    }

    // ====================================================================
    // OAuthSession: apply_refresh tests
    // ====================================================================

    #[test]
    fn apply_refresh_updates_tokens_with_expiry() {
        let mut session = make_session(None);
        let json = r#"{
            "access_token": "new-access-token",
            "token_type": "bearer",
            "expires_in": 3600,
            "refresh_token": "new-refresh-token"
        }"#;
        let response: BasicTokenResponse = serde_json::from_str(json).expect("should deserialize");
        let now = DateTime::parse_from_rfc3339("2025-06-01T12:00:00Z")
            .expect("parse")
            .to_utc();
        session.apply_refresh(&response, now);

        assert_eq!(session.access_token().secret(), "new-access-token");
        assert_eq!(session.refresh_token().secret(), "new-refresh-token");

        let expires_at = session.tokens.expires_at.expect("should have expiry");
        assert_eq!(
            expires_at,
            now + chrono::Duration::seconds(3600),
            "expires_at should be exactly now + 3600s"
        );
    }

    #[test]
    fn apply_refresh_updates_tokens_without_expiry() {
        let now = DateTime::parse_from_rfc3339("2025-06-01T12:00:00Z")
            .expect("parse")
            .to_utc();
        let mut session = make_session(Some(now));
        let json = r#"{
            "access_token": "new-at",
            "token_type": "bearer"
        }"#;
        let response: BasicTokenResponse = serde_json::from_str(json).expect("should deserialize");
        session.apply_refresh(&response, now);

        assert_eq!(session.access_token().secret(), "new-at");
        assert!(session.tokens.expires_at.is_none());
    }

    #[test]
    fn from_raw_creates_token_data() {
        let tokens = OAuthTokenData::from_raw(
            "access-token".into(),
            "refresh-token".into(),
            None,
            "bearer".into(),
            Some(vec!["OAuth2Read".into(), "OAuth2Write".into()]),
        );
        assert_eq!(tokens.access_token.secret(), "access-token");
        assert_eq!(tokens.refresh_token.secret(), "refresh-token");
        assert!(tokens.expires_at.is_none());
        assert_eq!(tokens.token_type, "bearer");
        assert_eq!(
            tokens.scopes,
            Some(vec!["OAuth2Read".into(), "OAuth2Write".into()])
        );
    }

    // ====================================================================
    // OAuth Login Flow Tests
    // ====================================================================

    fn test_login_config() -> OAuthLoginConfig {
        OAuthLoginConfig {
            client_id: "test-client-id".into(),
            redirect_uri: "http://127.0.0.1:18338/callback".into(),
            scopes: vec!["OAuth2Read".into(), "OAuth2Write".into()],
        }
    }

    #[test]
    fn build_authorize_url_contains_required_params() {
        let config = test_login_config();
        let state = CsrfToken::new("test-state-token".into());
        let (challenge, _verifier) = PkceCodeChallenge::new_random_sha256();

        let url_str = build_authorize_url(&config, &state, challenge);
        let url = url::Url::parse(&url_str).expect("should be a valid URL");

        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("oauth.onshape.com"));
        assert_eq!(url.path(), "/oauth/authorize");

        let params: std::collections::HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        assert_eq!(
            params.get("client_id").map(String::as_str),
            Some("test-client-id")
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:18338/callback")
        );
        assert_eq!(
            params.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(
            params.get("state").map(String::as_str),
            Some("test-state-token")
        );
        assert!(params.contains_key("code_challenge"));
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        // Scopes should be space-separated in the scope parameter.
        let scope = params.get("scope").expect("should have scope parameter");
        assert!(scope.contains("OAuth2Read"));
        assert!(scope.contains("OAuth2Write"));
    }

    #[test]
    fn build_authorize_url_with_no_scopes() {
        let config = OAuthLoginConfig {
            client_id: "cid".into(),
            redirect_uri: "http://127.0.0.1:18338/callback".into(),
            scopes: vec![],
        };
        let state = CsrfToken::new("state".into());
        let (challenge, _verifier) = PkceCodeChallenge::new_random_sha256();

        let url_str = build_authorize_url(&config, &state, challenge);
        let url = url::Url::parse(&url_str).expect("should be a valid URL");
        let params: std::collections::HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        // No scope parameter when no scopes requested.
        assert!(!params.contains_key("scope"));
    }

    #[test]
    fn validate_callback_extracts_code() {
        let state = CsrfToken::new("my-state".into());
        let callback = "http://127.0.0.1:18338/callback?code=auth-code-123&state=my-state";

        let code = validate_callback(callback, &state).expect("should validate");
        assert_eq!(code.secret(), "auth-code-123");
    }

    #[test]
    fn validate_callback_detects_state_mismatch() {
        let state = CsrfToken::new("expected-state".into());
        let callback = "http://127.0.0.1:18338/callback?code=abc&state=wrong-state";

        let err = validate_callback(callback, &state).expect_err("should fail");
        assert!(
            matches!(err, CallbackValidationError::StateMismatch { .. }),
            "expected StateMismatch, got: {err:?}"
        );
    }

    #[test]
    fn validate_callback_detects_oauth_error() {
        let state = CsrfToken::new("my-state".into());
        let callback = "http://127.0.0.1:18338/callback?error=access_denied&error_description=User+denied+access&state=my-state";

        let err = validate_callback(callback, &state).expect_err("should fail");
        match err {
            CallbackValidationError::OAuthError { error, description } => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("User denied access"));
            }
            other => panic!("expected OAuthError, got: {other:?}"),
        }
    }

    #[test]
    fn validate_callback_detects_missing_state() {
        let state = CsrfToken::new("my-state".into());
        let callback = "http://127.0.0.1:18338/callback?code=abc";

        let err = validate_callback(callback, &state).expect_err("should fail");
        assert!(
            matches!(err, CallbackValidationError::MissingState),
            "expected MissingState, got: {err:?}"
        );
    }

    #[test]
    fn validate_callback_detects_missing_code() {
        let state = CsrfToken::new("my-state".into());
        let callback = "http://127.0.0.1:18338/callback?state=my-state";

        let err = validate_callback(callback, &state).expect_err("should fail");
        assert!(
            matches!(err, CallbackValidationError::MissingCode),
            "expected MissingCode, got: {err:?}"
        );
    }

    #[test]
    fn validate_callback_detects_invalid_url() {
        let state = CsrfToken::new("my-state".into());
        let err = validate_callback("not a url at all ://", &state).expect_err("should fail");
        assert!(
            matches!(err, CallbackValidationError::InvalidUrl(_)),
            "expected InvalidUrl, got: {err:?}"
        );
    }
}
