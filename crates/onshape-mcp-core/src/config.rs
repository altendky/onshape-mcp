//! Configuration types and validation logic.
//!
//! Pure data types and validation for application configuration.
//! No I/O — config loading is handled by `onshape-mcp-io`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use onshape_client_core::auth::AuthMethod;
use secrecy::SecretString;
use serde::Deserialize;

/// Default timeout for HTTP requests to the Onshape API.
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Default interval for periodic credential validation checks.
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes

/// Minimum allowable interval for periodic credential validation checks.
///
/// Values below this threshold are clamped up during config loading
/// to prevent overly aggressive polling.
pub const MIN_CHECK_INTERVAL: Duration = Duration::from_secs(15);

// ============================================================================
// Configuration Types
// ============================================================================

/// Authentication configuration.
///
/// Contains optional credentials, auth method, and check interval settings.
/// Credentials are wrapped in [`SecretString`] to prevent accidental logging.
#[derive(Deserialize)]
pub struct AuthConfig {
    /// Onshape API access key (for Basic/HMAC auth).
    #[serde(default)]
    pub access_key: Option<SecretString>,
    /// Onshape API secret key (for Basic/HMAC auth).
    #[serde(default)]
    pub secret_key: Option<SecretString>,
    /// OAuth 2.0 client ID (for OAuth auth).
    #[serde(default)]
    pub client_id: Option<String>,
    /// OAuth 2.0 client secret (for OAuth auth).
    #[serde(default)]
    pub client_secret: Option<SecretString>,
    /// OAuth token exchange proxy URL (for proxy-based OAuth auth).
    ///
    /// When set, the server uses this proxy for token refresh instead of
    /// contacting Onshape directly. The proxy holds the client secret.
    /// Mutually exclusive with `client_secret` — use one or the other.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Authentication method to use for Onshape API requests.
    #[serde(default = "default_auth_method")]
    pub method: AuthMethod,
    /// Interval for periodic credential validation (default: 5 minutes).
    #[serde(
        default = "default_check_interval",
        deserialize_with = "deserialize_duration"
    )]
    pub check_interval: Duration,
}

/// Onshape API client configuration (request timeouts, etc.).
///
/// Previously named `HttpConfig` with TOML section `[http]`. Renamed to `[api]`
/// to avoid ambiguity with the new HTTP transport subcommand.
#[derive(Deserialize)]
pub struct ApiConfig {
    /// Request timeout for Onshape API calls (default: 30 seconds).
    #[serde(
        default = "default_http_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub timeout: Duration,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_HTTP_TIMEOUT,
        }
    }
}

/// Default host for the HTTP transport server.
pub const DEFAULT_TRANSPORT_HOST: &str = "127.0.0.1";

/// Default port for the HTTP transport server.
pub const DEFAULT_TRANSPORT_PORT: u16 = 8080;

/// HTTP transport configuration.
///
/// Used by the `onshape-mcp http` subcommand to serve the MCP server
/// over Streamable HTTP with per-user OAuth authentication.
#[derive(Deserialize)]
pub struct HttpTransportConfig {
    /// Listen address (default: `127.0.0.1`).
    #[serde(default = "default_transport_host")]
    pub host: String,
    /// Listen port (default: `8080`).
    #[serde(default = "default_transport_port")]
    pub port: u16,
    /// Public URL of the server (e.g. `https://mcp.example.com`).
    ///
    /// Required — used in OAuth metadata endpoint URLs. The server will
    /// fail at startup with a clear error if this is missing.
    #[serde(default)]
    pub public_url: Option<String>,
    /// Onshape OAuth application client ID.
    #[serde(default)]
    pub onshape_client_id: Option<String>,
    /// Onshape OAuth application client secret.
    #[serde(default)]
    pub onshape_client_secret: Option<SecretString>,
    /// Allowlist of Onshape user IDs permitted to connect.
    ///
    /// Empty list = fail-closed (nobody allowed).
    ///
    /// Supports two formats:
    /// - **TOML array of objects**: `[[http.allowed_users]]` with `id` and optional `name`
    /// - **Comma-separated string** (e.g. from env vars):
    ///   `id1:name1,id2:name2` or just `id1,id2` (names are optional)
    #[serde(default, deserialize_with = "deserialize_allowed_users")]
    pub allowed_users: Vec<AllowedUser>,
}

impl Default for HttpTransportConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_TRANSPORT_HOST.to_string(),
            port: DEFAULT_TRANSPORT_PORT,
            public_url: None,
            onshape_client_id: None,
            onshape_client_secret: None,
            allowed_users: Vec::new(),
        }
    }
}

/// An entry in the HTTP transport allowlist.
#[derive(Deserialize, Clone, Debug)]
pub struct AllowedUser {
    /// Onshape user ID (e.g. `6073e74c7f81d1054fca4373`).
    pub id: String,
    /// Human-readable name (ignored at runtime, for config readability).
    #[serde(default)]
    pub name: Option<String>,
}

/// Top-level application configuration.
#[derive(Default, Deserialize)]
pub struct AppConfig {
    /// Authentication settings.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Onshape API client settings (timeouts, etc.).
    #[serde(default)]
    pub api: ApiConfig,
    /// HTTP transport settings (for `onshape-mcp http` subcommand).
    #[serde(default)]
    pub http: HttpTransportConfig,
}

// ============================================================================
// Auth Resolution
// ============================================================================

/// Status of the OAuth token file, as probed by the I/O layer.
///
/// This is a lightweight summary of what was found on disk — no secrets,
/// just enough for the core to make decisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenStatus {
    /// No token file found (or the file could not be read).
    Absent,
    /// Token file exists and was successfully parsed.
    Present {
        /// When the access token expires, if known.
        expires_at: Option<DateTime<Utc>>,
        /// Whether the token file contains a proxy URL for token refresh.
        proxy_url: Option<String>,
    },
}

/// Summary of all available credential sources.
///
/// Built by the I/O layer from the config + token file probe.
/// Contains no secrets — just presence flags and token status.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct AuthInventory {
    /// Whether an API access key is configured.
    pub has_access_key: bool,
    /// Whether an API secret key is configured.
    pub has_secret_key: bool,
    /// Whether an OAuth client ID is configured.
    pub has_client_id: bool,
    /// Whether an OAuth client secret is configured.
    pub has_client_secret: bool,
    /// Whether an OAuth token exchange proxy URL is configured.
    pub has_proxy_url: bool,
    /// Status of the OAuth token file on disk.
    pub token_status: TokenStatus,
}

impl AuthInventory {
    /// Build an inventory from an [`AuthConfig`] and a [`TokenStatus`].
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn from_config(config: &AuthConfig, token_status: TokenStatus) -> Self {
        // proxy_url can come from config OR from the token file.
        let has_proxy_url = config.proxy_url.is_some()
            || matches!(
                &token_status,
                TokenStatus::Present {
                    proxy_url: Some(_),
                    ..
                }
            );

        Self {
            has_access_key: config.access_key.is_some(),
            has_secret_key: config.secret_key.is_some(),
            has_client_id: config.client_id.is_some(),
            has_client_secret: config.client_secret.is_some(),
            has_proxy_url,
            token_status,
        }
    }
}

/// The resolved authentication state after examining all credential sources.
///
/// Determined by [`resolve_auth`] from the configured method and available
/// credentials. This is what the auth status tool reports and what the I/O
/// layer uses to decide which `ApiState` variant to construct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedAuth {
    /// No usable credentials were found.
    NotConfigured {
        /// The configured auth method (for status reporting).
        configured_method: AuthMethod,
        /// Human-readable explanation of why nothing was configured.
        detail: String,
    },
    /// Basic (API key) auth is ready.
    Basic,
    /// OAuth with tokens — ready to make API calls.
    OAuthReady {
        /// When the access token expires, if known.
        expires_at: Option<DateTime<Utc>>,
    },
    /// OAuth client credentials are present but no tokens yet.
    ///
    /// The user needs to complete the OAuth authorization flow
    /// (e.g. via the `OpenCode` plugin) to obtain tokens.
    OAuthPending,
}

/// Resolve the authentication state from config method and available credentials.
///
/// This is a pure function — no I/O. The I/O layer provides the
/// [`AuthInventory`] (by probing config and token file), and this function
/// determines which auth state to use.
#[must_use]
pub fn resolve_auth(method: AuthMethod, inventory: &AuthInventory) -> ResolvedAuth {
    match method {
        AuthMethod::Auto => resolve_auto(inventory),
        AuthMethod::OAuth => resolve_oauth(method, inventory),
        // Basic and future API-key-based methods (e.g., HMAC).
        _ => resolve_basic(method, inventory),
    }
}

/// Whether the inventory has enough OAuth configuration to operate.
///
/// True when either:
/// - Direct mode: `client_id` + `client_secret` are both present
/// - Proxy mode: `proxy_url` is present (the proxy knows its own `client_id`/secret)
const fn has_oauth_capability(inventory: &AuthInventory) -> bool {
    (inventory.has_client_id && inventory.has_client_secret) || inventory.has_proxy_url
}

/// Auto-detect the best auth method from available credentials.
///
/// Priority order:
/// 1. OAuth with tokens (most secure, scoped, revocable)
/// 2. Basic auth (API keys present)
/// 3. OAuth pending (client creds but awaiting token)
/// 4. Not configured
fn resolve_auto(inventory: &AuthInventory) -> ResolvedAuth {
    let has_oauth = has_oauth_capability(inventory);

    // Priority 1: OAuth with tokens
    if has_oauth && let TokenStatus::Present { expires_at, .. } = &inventory.token_status {
        return ResolvedAuth::OAuthReady {
            expires_at: *expires_at,
        };
    }

    // Priority 2: Basic with both keys
    if inventory.has_access_key && inventory.has_secret_key {
        return ResolvedAuth::Basic;
    }

    // Priority 3: OAuth pending (client creds but no tokens)
    if has_oauth {
        return ResolvedAuth::OAuthPending;
    }

    // Nothing complete
    ResolvedAuth::NotConfigured {
        configured_method: AuthMethod::Auto,
        detail: not_configured_detail(AuthMethod::Auto, inventory),
    }
}

/// Resolve explicit Basic auth method.
fn resolve_basic(method: AuthMethod, inventory: &AuthInventory) -> ResolvedAuth {
    if inventory.has_access_key && inventory.has_secret_key {
        return ResolvedAuth::Basic;
    }
    ResolvedAuth::NotConfigured {
        configured_method: method,
        detail: not_configured_detail(method, inventory),
    }
}

/// Resolve explicit OAuth method.
fn resolve_oauth(method: AuthMethod, inventory: &AuthInventory) -> ResolvedAuth {
    let has_oauth = has_oauth_capability(inventory);

    if has_oauth {
        if let TokenStatus::Present { expires_at, .. } = &inventory.token_status {
            return ResolvedAuth::OAuthReady {
                expires_at: *expires_at,
            };
        }
        return ResolvedAuth::OAuthPending;
    }

    ResolvedAuth::NotConfigured {
        configured_method: method,
        detail: not_configured_detail(method, inventory),
    }
}

/// Build a human-readable detail message for the `NotConfigured` state.
fn not_configured_detail(method: AuthMethod, inventory: &AuthInventory) -> String {
    match method {
        AuthMethod::Auto => {
            let mut missing = Vec::new();
            if !inventory.has_access_key && !inventory.has_secret_key {
                missing.push("API keys (access_key + secret_key)");
            } else if !inventory.has_access_key {
                missing.push("access_key");
            } else if !inventory.has_secret_key {
                missing.push("secret_key");
            }
            if !inventory.has_client_id && !inventory.has_client_secret {
                missing.push("OAuth credentials (client_id + client_secret)");
            } else if !inventory.has_client_id {
                missing.push("client_id");
            } else if !inventory.has_client_secret {
                missing.push("client_secret");
            }
            if missing.is_empty() {
                "No credentials configured".into()
            } else {
                format!(
                    "No complete credentials found. Missing: {}",
                    missing.join(", ")
                )
            }
        }
        AuthMethod::OAuth => {
            if inventory.has_proxy_url {
                // proxy_url is set — this shouldn't reach NotConfigured, but
                // handle gracefully.
                "OAuth proxy configured but tokens not available".into()
            } else if !inventory.has_client_id && !inventory.has_client_secret {
                "No credentials configured (set client_id + client_secret, or proxy_url)".into()
            } else if !inventory.has_client_id {
                "Incomplete credentials: client_id is not configured".into()
            } else {
                "Incomplete credentials: client_secret is not configured (or set proxy_url)".into()
            }
        }
        // Basic and any future API-key-based methods
        _ => {
            if !inventory.has_access_key && !inventory.has_secret_key {
                "No credentials configured".into()
            } else if !inventory.has_access_key {
                "Incomplete credentials: access_key is not configured".into()
            } else {
                "Incomplete credentials: secret_key is not configured".into()
            }
        }
    }
}

impl AuthConfig {
    /// Clamps `check_interval` to at least [`MIN_CHECK_INTERVAL`].
    ///
    /// Returns `Some(original)` if the value was below the minimum and was
    /// clamped up, or `None` if no change was needed. Callers should use
    /// the returned original value to emit a warning.
    pub fn clamp_check_interval(&mut self) -> Option<Duration> {
        if self.check_interval < MIN_CHECK_INTERVAL {
            let original = self.check_interval;
            self.check_interval = MIN_CHECK_INTERVAL;
            Some(original)
        } else {
            None
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            proxy_url: None,
            method: AuthMethod::Auto,
            check_interval: DEFAULT_CHECK_INTERVAL,
        }
    }
}

// ============================================================================
// Serde Helpers
// ============================================================================

/// Default auth method for serde deserialization.
const fn default_auth_method() -> AuthMethod {
    AuthMethod::Auto
}

/// Default check interval for serde deserialization.
const fn default_check_interval() -> Duration {
    DEFAULT_CHECK_INTERVAL
}

/// Default HTTP timeout for serde deserialization.
const fn default_http_timeout() -> Duration {
    DEFAULT_HTTP_TIMEOUT
}

/// Default host for the HTTP transport.
fn default_transport_host() -> String {
    DEFAULT_TRANSPORT_HOST.to_string()
}

/// Default port for the HTTP transport.
const fn default_transport_port() -> u16 {
    DEFAULT_TRANSPORT_PORT
}

/// Deserializes a duration from either an integer (seconds) or a string like "5m", "300s".
///
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours).
/// A bare integer is treated as seconds.
fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    /// Visitor that handles both integer and string representations of durations.
    struct DurationVisitor;

    impl de::Visitor<'_> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(
                "a duration as seconds (integer) or string like \"5m\", \"300s\", \"1h\"",
            )
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<Duration, E> {
            Ok(Duration::from_secs(value))
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<Duration, E> {
            u64::try_from(value)
                .map(Duration::from_secs)
                .map_err(|_| de::Error::custom("duration must be non-negative"))
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Duration, E> {
            parse_duration_str(value).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(DurationVisitor)
}

/// Parses a duration string like "5m", "300s", "1h", or bare seconds.
fn parse_duration_str(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".into());
    }

    // Try parsing as bare integer (seconds)
    if let Ok(secs) = s.parse::<u64>() {
        return Ok(Duration::from_secs(secs));
    }

    // Parse with suffix
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else {
        return Err(format!(
            "invalid duration \"{s}\": expected a number with optional suffix (s, m, h)"
        ));
    };

    let num: u64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("invalid duration \"{s}\": numeric part is not a valid integer"))?;

    num.checked_mul(multiplier)
        .map(Duration::from_secs)
        .ok_or_else(|| format!("invalid duration \"{s}\": value overflows"))
}

/// Deserializes `allowed_users` from either a TOML array of objects or a
/// comma-separated string (useful for environment variables).
///
/// String format: `id1:name1,id2:name2` or just `id1,id2`.
/// The `:name` portion is optional and ignored at runtime.
fn deserialize_allowed_users<'de, D>(deserializer: D) -> Result<Vec<AllowedUser>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct AllowedUsersVisitor;

    impl<'de> de::Visitor<'de> for AllowedUsersVisitor {
        type Value = Vec<AllowedUser>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(
                "a list of allowed users (TOML array of {id, name} objects) \
                 or a comma-separated string like \"id1:name1,id2:name2\"",
            )
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Vec<AllowedUser>, E> {
            Ok(parse_allowed_users_csv(value))
        }

        fn visit_seq<A: de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Vec<AllowedUser>, A::Error> {
            let mut users = Vec::new();
            while let Some(user) = seq.next_element()? {
                users.push(user);
            }
            Ok(users)
        }
    }

    deserializer.deserialize_any(AllowedUsersVisitor)
}

/// Parse a comma-separated string of `id:name` pairs into `AllowedUser` entries.
///
/// - Empty or whitespace-only strings produce an empty vec.
/// - Each entry is trimmed. Empty entries (from trailing commas) are skipped.
/// - The `:name` portion is optional.
pub fn parse_allowed_users_csv(s: &str) -> Vec<AllowedUser> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    s.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            if let Some((id, name)) = entry.split_once(':') {
                AllowedUser {
                    id: id.trim().to_string(),
                    name: Some(name.trim().to_string()),
                }
            } else {
                AllowedUser {
                    id: entry.to_string(),
                    name: None,
                }
            }
        })
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    // ====================================================================
    // Auth Resolution Tests
    // ====================================================================

    fn inventory_nothing() -> AuthInventory {
        AuthInventory {
            has_access_key: false,
            has_secret_key: false,
            has_client_id: false,
            has_client_secret: false,
            has_proxy_url: false,
            token_status: TokenStatus::Absent,
        }
    }

    fn inventory_basic() -> AuthInventory {
        AuthInventory {
            has_access_key: true,
            has_secret_key: true,
            ..inventory_nothing()
        }
    }

    fn inventory_oauth_with_tokens() -> AuthInventory {
        AuthInventory {
            has_client_id: true,
            has_client_secret: true,
            token_status: TokenStatus::Present {
                expires_at: None,
                proxy_url: None,
            },
            ..inventory_nothing()
        }
    }

    fn inventory_oauth_no_tokens() -> AuthInventory {
        AuthInventory {
            has_client_id: true,
            has_client_secret: true,
            token_status: TokenStatus::Absent,
            ..inventory_nothing()
        }
    }

    // --- Auto resolution ---

    #[test]
    fn auto_with_nothing_returns_not_configured() {
        let result = resolve_auth(AuthMethod::Auto, &inventory_nothing());
        assert!(matches!(result, ResolvedAuth::NotConfigured { .. }));
    }

    #[test]
    fn auto_with_basic_keys_returns_basic() {
        let result = resolve_auth(AuthMethod::Auto, &inventory_basic());
        assert_eq!(result, ResolvedAuth::Basic);
    }

    #[test]
    fn auto_with_oauth_tokens_returns_oauth_ready() {
        let result = resolve_auth(AuthMethod::Auto, &inventory_oauth_with_tokens());
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    #[test]
    fn auto_with_oauth_no_tokens_returns_pending() {
        let result = resolve_auth(AuthMethod::Auto, &inventory_oauth_no_tokens());
        assert_eq!(result, ResolvedAuth::OAuthPending);
    }

    #[test]
    fn auto_oauth_wins_over_basic_when_tokens_present() {
        let inv = AuthInventory {
            has_access_key: true,
            has_secret_key: true,
            has_client_id: true,
            has_client_secret: true,
            has_proxy_url: false,
            token_status: TokenStatus::Present {
                expires_at: None,
                proxy_url: None,
            },
        };
        let result = resolve_auth(AuthMethod::Auto, &inv);
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    #[test]
    fn auto_basic_wins_over_oauth_pending() {
        let inv = AuthInventory {
            has_access_key: true,
            has_secret_key: true,
            has_client_id: true,
            has_client_secret: true,
            has_proxy_url: false,
            token_status: TokenStatus::Absent,
        };
        let result = resolve_auth(AuthMethod::Auto, &inv);
        assert_eq!(result, ResolvedAuth::Basic);
    }

    #[test]
    fn auto_partial_basic_falls_through_to_not_configured() {
        let inv = AuthInventory {
            has_access_key: true,
            has_secret_key: false,
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::Auto, &inv);
        assert!(matches!(result, ResolvedAuth::NotConfigured { .. }));
    }

    #[test]
    fn auto_partial_oauth_falls_through_to_not_configured() {
        let inv = AuthInventory {
            has_client_id: true,
            has_client_secret: false,
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::Auto, &inv);
        assert!(matches!(result, ResolvedAuth::NotConfigured { .. }));
    }

    // --- Explicit Basic resolution ---

    #[test]
    fn basic_with_keys_returns_basic() {
        let result = resolve_auth(AuthMethod::Basic, &inventory_basic());
        assert_eq!(result, ResolvedAuth::Basic);
    }

    #[test]
    fn basic_without_keys_returns_not_configured() {
        let result = resolve_auth(AuthMethod::Basic, &inventory_nothing());
        assert!(matches!(
            result,
            ResolvedAuth::NotConfigured {
                configured_method: AuthMethod::Basic,
                ..
            }
        ));
    }

    #[test]
    fn basic_missing_secret_key_reports_it() {
        let inv = AuthInventory {
            has_access_key: true,
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::Basic, &inv);
        match result {
            ResolvedAuth::NotConfigured { detail, .. } => {
                assert!(detail.contains("secret_key"));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn basic_missing_access_key_reports_it() {
        let inv = AuthInventory {
            has_secret_key: true,
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::Basic, &inv);
        match result {
            ResolvedAuth::NotConfigured { detail, .. } => {
                assert!(detail.contains("access_key"));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    // --- Explicit OAuth resolution ---

    #[test]
    fn oauth_with_tokens_returns_ready() {
        let result = resolve_auth(AuthMethod::OAuth, &inventory_oauth_with_tokens());
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    #[test]
    fn oauth_without_tokens_returns_pending() {
        let result = resolve_auth(AuthMethod::OAuth, &inventory_oauth_no_tokens());
        assert_eq!(result, ResolvedAuth::OAuthPending);
    }

    #[test]
    fn oauth_without_client_creds_returns_not_configured() {
        let result = resolve_auth(AuthMethod::OAuth, &inventory_nothing());
        assert!(matches!(
            result,
            ResolvedAuth::NotConfigured {
                configured_method: AuthMethod::OAuth,
                ..
            }
        ));
    }

    #[test]
    fn oauth_missing_client_secret_reports_it() {
        let inv = AuthInventory {
            has_client_id: true,
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::OAuth, &inv);
        match result {
            ResolvedAuth::NotConfigured { detail, .. } => {
                assert!(detail.contains("client_secret"));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn oauth_missing_client_id_reports_it() {
        let inv = AuthInventory {
            has_client_secret: true,
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::OAuth, &inv);
        match result {
            ResolvedAuth::NotConfigured { detail, .. } => {
                assert!(detail.contains("client_id"));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    // ====================================================================
    // AuthConfig Default Tests
    // ====================================================================

    #[test]
    fn default_auth_config() {
        let config = AuthConfig::default();
        assert!(config.access_key.is_none());
        assert!(config.secret_key.is_none());
        assert_eq!(config.method, AuthMethod::Auto);
        assert_eq!(config.check_interval, Duration::from_secs(300));
    }

    // ====================================================================
    // Duration Parsing Tests
    // ====================================================================

    #[test]
    fn parse_duration_seconds_integer() {
        assert_eq!(
            parse_duration_str("300").expect("should parse"),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn parse_duration_seconds_suffix() {
        assert_eq!(
            parse_duration_str("300s").expect("should parse"),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(
            parse_duration_str("5m").expect("should parse"),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(
            parse_duration_str("1h").expect("should parse"),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn parse_duration_empty_fails() {
        assert!(parse_duration_str("").is_err());
    }

    #[test]
    fn parse_duration_invalid_suffix_fails() {
        assert!(parse_duration_str("5x").is_err());
    }

    #[test]
    fn parse_duration_not_a_number_fails() {
        assert!(parse_duration_str("abcm").is_err());
    }

    #[test]
    fn parse_duration_overflow_fails() {
        assert!(parse_duration_str("5124095576030432h").is_err());
    }

    // ====================================================================
    // TOML Deserialization Tests
    // ====================================================================

    #[test]
    fn deserialize_negative_integer_interval_fails() {
        let toml_str = r#"
            access_key = "ak"
            secret_key = "sk"
            check_interval = -5
        "#;

        let result: Result<AuthConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_auth_config_from_toml() {
        let toml_str = r#"
            access_key = "my-access-key"
            secret_key = "my-secret-key"
            check_interval = "10m"
        "#;

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(
            config
                .access_key
                .as_ref()
                .expect("should have access_key")
                .expose_secret(),
            "my-access-key"
        );
        assert_eq!(
            config
                .secret_key
                .as_ref()
                .expect("should have secret_key")
                .expose_secret(),
            "my-secret-key"
        );
        assert_eq!(config.check_interval, Duration::from_secs(600));
    }

    #[test]
    fn deserialize_auth_config_integer_interval() {
        let toml_str = r#"
            access_key = "ak"
            secret_key = "sk"
            check_interval = 120
        "#;

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.check_interval, Duration::from_secs(120));
    }

    #[test]
    fn deserialize_auth_config_defaults() {
        let toml_str = "";

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert!(config.access_key.is_none());
        assert!(config.secret_key.is_none());
        assert_eq!(config.method, AuthMethod::Auto);
        assert_eq!(config.check_interval, Duration::from_secs(300));
    }

    #[test]
    fn deserialize_auth_config_method_basic() {
        let toml_str = r#"
            method = "basic"
        "#;

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.method, AuthMethod::Basic);
    }

    #[test]
    fn deserialize_auth_config_method_auto() {
        let toml_str = r#"
            method = "auto"
        "#;

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.method, AuthMethod::Auto);
    }

    #[test]
    fn deserialize_auth_config_invalid_method_fails() {
        let toml_str = r#"
            method = "unknown_method"
        "#;

        let result: Result<AuthConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_app_config_with_auth_section() {
        let toml_str = r#"
            [auth]
            access_key = "ak"
            secret_key = "sk"
        "#;

        let config: AppConfig = toml::from_str(toml_str).expect("should deserialize");
        let inv = AuthInventory::from_config(&config.auth, TokenStatus::Absent);
        assert_eq!(resolve_auth(config.auth.method, &inv), ResolvedAuth::Basic);
    }

    #[test]
    fn deserialize_app_config_empty() {
        let toml_str = "";

        let config: AppConfig = toml::from_str(toml_str).expect("should deserialize");
        let inv = AuthInventory::from_config(&config.auth, TokenStatus::Absent);
        let result = resolve_auth(config.auth.method, &inv);
        assert!(matches!(result, ResolvedAuth::NotConfigured { .. }));
    }

    // ====================================================================
    // Check Interval Clamping Tests
    // ====================================================================

    #[test]
    fn clamp_check_interval_below_minimum() {
        let mut config = AuthConfig {
            check_interval: Duration::from_secs(0),
            ..AuthConfig::default()
        };
        let original = config.clamp_check_interval();
        assert_eq!(original, Some(Duration::from_secs(0)));
        assert_eq!(config.check_interval, MIN_CHECK_INTERVAL);
    }

    #[test]
    fn clamp_check_interval_just_below_minimum() {
        let mut config = AuthConfig {
            check_interval: Duration::from_secs(14),
            ..AuthConfig::default()
        };
        let original = config.clamp_check_interval();
        assert_eq!(original, Some(Duration::from_secs(14)));
        assert_eq!(config.check_interval, MIN_CHECK_INTERVAL);
    }

    #[test]
    fn clamp_check_interval_at_minimum_unchanged() {
        let mut config = AuthConfig {
            check_interval: MIN_CHECK_INTERVAL,
            ..AuthConfig::default()
        };
        let original = config.clamp_check_interval();
        assert_eq!(original, None);
        assert_eq!(config.check_interval, MIN_CHECK_INTERVAL);
    }

    #[test]
    fn clamp_check_interval_above_minimum_unchanged() {
        let mut config = AuthConfig {
            check_interval: Duration::from_secs(300),
            ..AuthConfig::default()
        };
        let original = config.clamp_check_interval();
        assert_eq!(original, None);
        assert_eq!(config.check_interval, Duration::from_secs(300));
    }

    // ====================================================================
    // OAuth TOML Deserialization Tests
    // ====================================================================

    #[test]
    fn deserialize_auth_config_method_oauth() {
        let toml_str = r#"
            method = "oauth"
            client_id = "my-client-id"
            client_secret = "my-client-secret"
        "#;

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.method, AuthMethod::OAuth);
        assert_eq!(config.client_id.as_deref(), Some("my-client-id"));
        assert_eq!(
            config
                .client_secret
                .as_ref()
                .expect("should have client_secret")
                .expose_secret(),
            "my-client-secret"
        );
    }

    #[test]
    fn deserialize_auth_config_oauth_defaults() {
        let toml_str = r#"
            method = "oauth"
        "#;

        let config: AuthConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.method, AuthMethod::OAuth);
        assert!(config.client_id.is_none());
        assert!(config.client_secret.is_none());
    }

    // ====================================================================
    // ApiConfig Tests
    // ====================================================================

    #[test]
    fn default_api_config() {
        let config = ApiConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
    }

    #[test]
    fn deserialize_api_config_with_timeout() {
        let toml_str = r#"
            timeout = "10s"
        "#;
        let config: ApiConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, Duration::from_secs(10));
    }

    #[test]
    fn deserialize_api_config_timeout_minutes() {
        let toml_str = r#"
            timeout = "2m"
        "#;
        let config: ApiConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, Duration::from_secs(120));
    }

    #[test]
    fn deserialize_api_config_timeout_integer() {
        let toml_str = r"
            timeout = 45
        ";
        let config: ApiConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, Duration::from_secs(45));
    }

    #[test]
    fn deserialize_api_config_defaults() {
        let toml_str = "";
        let config: ApiConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, DEFAULT_HTTP_TIMEOUT);
    }

    #[test]
    fn deserialize_app_config_with_api_section() {
        let toml_str = r#"
            [auth]
            access_key = "ak"
            secret_key = "sk"

            [api]
            timeout = "60s"
        "#;

        let config: AppConfig = toml::from_str(toml_str).expect("should deserialize");
        let inv = AuthInventory::from_config(&config.auth, TokenStatus::Absent);
        assert_eq!(resolve_auth(config.auth.method, &inv), ResolvedAuth::Basic);
        assert_eq!(config.api.timeout, Duration::from_secs(60));
    }

    // ====================================================================
    // HttpTransportConfig Tests
    // ====================================================================

    #[test]
    fn default_http_transport_config() {
        let config = HttpTransportConfig::default();
        assert_eq!(config.host, DEFAULT_TRANSPORT_HOST);
        assert_eq!(config.port, DEFAULT_TRANSPORT_PORT);
        assert!(config.public_url.is_none());
        assert!(config.onshape_client_id.is_none());
        assert!(config.onshape_client_secret.is_none());
        assert!(config.allowed_users.is_empty());
    }

    #[test]
    fn deserialize_http_transport_config_defaults() {
        let toml_str = "";
        let config: HttpTransportConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.host, DEFAULT_TRANSPORT_HOST);
        assert_eq!(config.port, DEFAULT_TRANSPORT_PORT);
        assert!(config.public_url.is_none());
        assert!(config.onshape_client_id.is_none());
        assert!(config.onshape_client_secret.is_none());
        assert!(config.allowed_users.is_empty());
    }

    #[test]
    fn deserialize_http_transport_config_full() {
        let toml_str = r#"
            host = "0.0.0.0"
            port = 9090
            public_url = "https://mcp.example.com"
            onshape_client_id = "my-client-id"
            onshape_client_secret = "my-secret"
            allowed_users = "abc123:Alice,def456:Bob"
        "#;
        let config: HttpTransportConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9090);
        assert_eq!(
            config.public_url.as_deref(),
            Some("https://mcp.example.com")
        );
        assert_eq!(config.onshape_client_id.as_deref(), Some("my-client-id"));
        assert_eq!(
            config
                .onshape_client_secret
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            Some("my-secret".to_string())
        );
        assert_eq!(config.allowed_users.len(), 2);
        assert_eq!(config.allowed_users[0].id, "abc123");
        assert_eq!(config.allowed_users[0].name.as_deref(), Some("Alice"));
        assert_eq!(config.allowed_users[1].id, "def456");
        assert_eq!(config.allowed_users[1].name.as_deref(), Some("Bob"));
    }

    #[test]
    fn deserialize_app_config_with_http_section() {
        let toml_str = r#"
            [http]
            host = "0.0.0.0"
            port = 3000
            public_url = "https://example.com"
        "#;
        let config: AppConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.http.host, "0.0.0.0");
        assert_eq!(config.http.port, 3000);
        assert_eq!(
            config.http.public_url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn deserialize_http_transport_config_allowed_users_toml_array() {
        let toml_str = r#"
            [[allowed_users]]
            id = "user1"
            name = "User One"

            [[allowed_users]]
            id = "user2"
        "#;
        let config: HttpTransportConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.allowed_users.len(), 2);
        assert_eq!(config.allowed_users[0].id, "user1");
        assert_eq!(config.allowed_users[0].name.as_deref(), Some("User One"));
        assert_eq!(config.allowed_users[1].id, "user2");
        assert!(config.allowed_users[1].name.is_none());
    }

    // ====================================================================
    // AuthInventory Construction Tests
    // ====================================================================

    #[test]
    fn inventory_from_config_with_basic_keys() {
        let config = AuthConfig {
            access_key: Some(SecretString::from("ak")),
            secret_key: Some(SecretString::from("sk")),
            ..AuthConfig::default()
        };
        let inv = AuthInventory::from_config(&config, TokenStatus::Absent);
        assert!(inv.has_access_key);
        assert!(inv.has_secret_key);
        assert!(!inv.has_client_id);
        assert!(!inv.has_client_secret);
        assert_eq!(inv.token_status, TokenStatus::Absent);
    }

    #[test]
    fn inventory_from_config_with_oauth_creds() {
        let config = AuthConfig {
            client_id: Some("cid".into()),
            client_secret: Some(SecretString::from("cs")),
            method: AuthMethod::OAuth,
            ..AuthConfig::default()
        };
        let inv = AuthInventory::from_config(
            &config,
            TokenStatus::Present {
                expires_at: None,
                proxy_url: None,
            },
        );
        assert!(!inv.has_access_key);
        assert!(!inv.has_secret_key);
        assert!(inv.has_client_id);
        assert!(inv.has_client_secret);
        assert!(matches!(inv.token_status, TokenStatus::Present { .. }));
    }

    // ====================================================================
    // Proxy URL Auth Resolution Tests
    // ====================================================================

    fn inventory_proxy_with_tokens() -> AuthInventory {
        AuthInventory {
            has_proxy_url: true,
            token_status: TokenStatus::Present {
                expires_at: None,
                proxy_url: Some("https://proxy.example.com".into()),
            },
            ..inventory_nothing()
        }
    }

    fn inventory_proxy_no_tokens() -> AuthInventory {
        AuthInventory {
            has_proxy_url: true,
            token_status: TokenStatus::Absent,
            ..inventory_nothing()
        }
    }

    #[test]
    fn auto_proxy_with_tokens_returns_oauth_ready() {
        let result = resolve_auth(AuthMethod::Auto, &inventory_proxy_with_tokens());
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    #[test]
    fn auto_proxy_without_tokens_returns_pending() {
        let result = resolve_auth(AuthMethod::Auto, &inventory_proxy_no_tokens());
        assert_eq!(result, ResolvedAuth::OAuthPending);
    }

    #[test]
    fn oauth_proxy_with_tokens_returns_ready() {
        let result = resolve_auth(AuthMethod::OAuth, &inventory_proxy_with_tokens());
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    #[test]
    fn oauth_proxy_without_tokens_returns_pending() {
        let result = resolve_auth(AuthMethod::OAuth, &inventory_proxy_no_tokens());
        assert_eq!(result, ResolvedAuth::OAuthPending);
    }

    #[test]
    fn proxy_url_without_client_secret_resolves_to_oauth() {
        // proxy_url alone is sufficient — no client_id or client_secret needed.
        let inv = AuthInventory {
            has_proxy_url: true,
            token_status: TokenStatus::Present {
                expires_at: None,
                proxy_url: None,
            },
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::Auto, &inv);
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    #[test]
    fn inventory_from_config_detects_proxy_url_in_config() {
        let config = AuthConfig {
            proxy_url: Some("https://proxy.example.com".into()),
            ..AuthConfig::default()
        };
        let inv = AuthInventory::from_config(&config, TokenStatus::Absent);
        assert!(inv.has_proxy_url);
    }

    #[test]
    fn inventory_from_config_detects_proxy_url_in_token_file() {
        let config = AuthConfig::default();
        let token_status = TokenStatus::Present {
            expires_at: None,
            proxy_url: Some("https://proxy.example.com".into()),
        };
        let inv = AuthInventory::from_config(&config, token_status);
        assert!(inv.has_proxy_url);
    }

    #[test]
    fn inventory_from_config_no_proxy_url_anywhere() {
        let config = AuthConfig::default();
        let inv = AuthInventory::from_config(&config, TokenStatus::Absent);
        assert!(!inv.has_proxy_url);
    }

    #[test]
    fn oauth_missing_client_secret_with_proxy_url_succeeds() {
        // Only client_id is set (no client_secret), but proxy_url is set.
        let inv = AuthInventory {
            has_client_id: true,
            has_proxy_url: true,
            token_status: TokenStatus::Present {
                expires_at: None,
                proxy_url: None,
            },
            ..inventory_nothing()
        };
        let result = resolve_auth(AuthMethod::OAuth, &inv);
        assert!(matches!(result, ResolvedAuth::OAuthReady { .. }));
    }

    // ====================================================================
    // Allowed Users CSV Parsing Tests
    // ====================================================================

    #[test]
    fn allowed_users_csv_with_names() {
        let users = parse_allowed_users_csv("abc123:alice,def456:bob");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].id, "abc123");
        assert_eq!(users[0].name.as_deref(), Some("alice"));
        assert_eq!(users[1].id, "def456");
        assert_eq!(users[1].name.as_deref(), Some("bob"));
    }

    #[test]
    fn allowed_users_csv_without_names() {
        let users = parse_allowed_users_csv("abc123,def456");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].id, "abc123");
        assert!(users[0].name.is_none());
        assert_eq!(users[1].id, "def456");
        assert!(users[1].name.is_none());
    }

    #[test]
    fn allowed_users_csv_mixed() {
        let users = parse_allowed_users_csv("abc123:alice,def456");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].id, "abc123");
        assert_eq!(users[0].name.as_deref(), Some("alice"));
        assert_eq!(users[1].id, "def456");
        assert!(users[1].name.is_none());
    }

    #[test]
    fn allowed_users_csv_empty_string() {
        let users = parse_allowed_users_csv("");
        assert!(users.is_empty());
    }

    #[test]
    fn allowed_users_csv_whitespace_only() {
        let users = parse_allowed_users_csv("   ");
        assert!(users.is_empty());
    }

    #[test]
    fn allowed_users_csv_with_whitespace() {
        let users = parse_allowed_users_csv(" abc123 : alice , def456 : bob ");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].id, "abc123");
        assert_eq!(users[0].name.as_deref(), Some("alice"));
        assert_eq!(users[1].id, "def456");
        assert_eq!(users[1].name.as_deref(), Some("bob"));
    }

    #[test]
    fn allowed_users_csv_trailing_comma() {
        let users = parse_allowed_users_csv("abc123:alice,");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "abc123");
    }

    #[test]
    fn allowed_users_csv_single_entry() {
        let users = parse_allowed_users_csv("60a1b2c3d4e5f60708091011:altendky");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "60a1b2c3d4e5f60708091011");
        assert_eq!(users[0].name.as_deref(), Some("altendky"));
    }
}
