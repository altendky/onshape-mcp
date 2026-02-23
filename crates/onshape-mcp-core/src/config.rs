//! Configuration types and validation logic.
//!
//! Pure data types and validation for application configuration.
//! No I/O — config loading is handled by `onshape-mcp-io`.

use std::time::Duration;

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

/// HTTP client configuration.
#[derive(Deserialize)]
pub struct HttpConfig {
    /// Request timeout for Onshape API calls (default: 30 seconds).
    #[serde(
        default = "default_http_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub timeout: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_HTTP_TIMEOUT,
        }
    }
}

/// Top-level application configuration.
#[derive(Default, Deserialize)]
pub struct AppConfig {
    /// Authentication settings.
    #[serde(default)]
    pub auth: AuthConfig,
    /// HTTP client settings.
    #[serde(default)]
    pub http: HttpConfig,
}

// ============================================================================
// Credential Status
// ============================================================================

/// Result of checking whether credentials are present in the configuration.
///
/// This is a pure check of the config data — it does not validate
/// credentials against the Onshape API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialStatus {
    /// Both access key and secret key are provided (API key auth).
    BothPresent,
    /// No credentials are configured.
    NonePresent,
    /// Only one credential is provided — the other is missing.
    Partial {
        /// Name of the missing credential field.
        missing: &'static str,
    },
    /// OAuth client credentials are fully configured (`client_id` + `client_secret`).
    OAuthConfigured,
    /// OAuth client credentials are partially configured.
    OAuthPartial {
        /// Name of the missing OAuth credential field.
        missing: &'static str,
    },
}

impl AuthConfig {
    /// Checks whether credentials are present (without validating them).
    ///
    /// For OAuth, checks `client_id` and `client_secret` instead of API keys.
    /// For Basic (and future HMAC), checks `access_key` and `secret_key`.
    #[must_use]
    pub const fn credential_status(&self) -> CredentialStatus {
        match self.method {
            AuthMethod::OAuth => match (&self.client_id, &self.client_secret) {
                (Some(_), Some(_)) => CredentialStatus::OAuthConfigured,
                (None, None) => CredentialStatus::NonePresent,
                (Some(_), None) => CredentialStatus::OAuthPartial {
                    missing: "client_secret",
                },
                (None, Some(_)) => CredentialStatus::OAuthPartial {
                    missing: "client_id",
                },
            },
            // Basic auth (and any future API-key-based methods) checks access_key/secret_key.
            _ => match (&self.access_key, &self.secret_key) {
                (Some(_), Some(_)) => CredentialStatus::BothPresent,
                (None, None) => CredentialStatus::NonePresent,
                (Some(_), None) => CredentialStatus::Partial {
                    missing: "secret_key",
                },
                (None, Some(_)) => CredentialStatus::Partial {
                    missing: "access_key",
                },
            },
        }
    }

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
            method: default_auth_method(),
            check_interval: DEFAULT_CHECK_INTERVAL,
        }
    }
}

// ============================================================================
// Serde Helpers
// ============================================================================

/// Default auth method for serde deserialization.
const fn default_auth_method() -> AuthMethod {
    AuthMethod::Basic
}

/// Default check interval for serde deserialization.
const fn default_check_interval() -> Duration {
    DEFAULT_CHECK_INTERVAL
}

/// Default HTTP timeout for serde deserialization.
const fn default_http_timeout() -> Duration {
    DEFAULT_HTTP_TIMEOUT
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn credential_status_both_present() {
        let config = AuthConfig {
            access_key: Some(SecretString::from("ak")),
            secret_key: Some(SecretString::from("sk")),
            ..AuthConfig::default()
        };
        assert_eq!(config.credential_status(), CredentialStatus::BothPresent);
    }

    #[test]
    fn credential_status_none_present() {
        let config = AuthConfig::default();
        assert_eq!(config.credential_status(), CredentialStatus::NonePresent);
    }

    #[test]
    fn credential_status_missing_secret_key() {
        let config = AuthConfig {
            access_key: Some(SecretString::from("ak")),
            secret_key: None,
            ..AuthConfig::default()
        };
        assert_eq!(
            config.credential_status(),
            CredentialStatus::Partial {
                missing: "secret_key"
            }
        );
    }

    #[test]
    fn credential_status_missing_access_key() {
        let config = AuthConfig {
            access_key: None,
            secret_key: Some(SecretString::from("sk")),
            ..AuthConfig::default()
        };
        assert_eq!(
            config.credential_status(),
            CredentialStatus::Partial {
                missing: "access_key"
            }
        );
    }

    #[test]
    fn default_auth_config() {
        let config = AuthConfig::default();
        assert!(config.access_key.is_none());
        assert!(config.secret_key.is_none());
        assert_eq!(config.method, AuthMethod::Basic);
        assert_eq!(config.check_interval, Duration::from_secs(300));
    }

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
        // A value that parses as a valid u64 but overflows when multiplied by 3600
        assert!(parse_duration_str("5124095576030432h").is_err());
    }

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
        assert_eq!(config.method, AuthMethod::Basic);
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
        assert_eq!(
            config.auth.credential_status(),
            CredentialStatus::BothPresent
        );
    }

    #[test]
    fn deserialize_app_config_empty() {
        let toml_str = "";

        let config: AppConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(
            config.auth.credential_status(),
            CredentialStatus::NonePresent
        );
    }

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
    // OAuth Credential Status Tests
    // ====================================================================

    #[test]
    fn credential_status_oauth_configured() {
        let config = AuthConfig {
            client_id: Some("my-client-id".into()),
            client_secret: Some(SecretString::from("my-client-secret")),
            method: AuthMethod::OAuth,
            ..AuthConfig::default()
        };
        assert_eq!(
            config.credential_status(),
            CredentialStatus::OAuthConfigured
        );
    }

    #[test]
    fn credential_status_oauth_none_present() {
        let config = AuthConfig {
            method: AuthMethod::OAuth,
            ..AuthConfig::default()
        };
        assert_eq!(config.credential_status(), CredentialStatus::NonePresent);
    }

    #[test]
    fn credential_status_oauth_missing_client_secret() {
        let config = AuthConfig {
            client_id: Some("my-client-id".into()),
            client_secret: None,
            method: AuthMethod::OAuth,
            ..AuthConfig::default()
        };
        assert_eq!(
            config.credential_status(),
            CredentialStatus::OAuthPartial {
                missing: "client_secret"
            }
        );
    }

    #[test]
    fn credential_status_oauth_missing_client_id() {
        let config = AuthConfig {
            client_id: None,
            client_secret: Some(SecretString::from("my-client-secret")),
            method: AuthMethod::OAuth,
            ..AuthConfig::default()
        };
        assert_eq!(
            config.credential_status(),
            CredentialStatus::OAuthPartial {
                missing: "client_id"
            }
        );
    }

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

    // --- HttpConfig tests ---

    #[test]
    fn default_http_config() {
        let config = HttpConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
    }

    #[test]
    fn deserialize_http_config_with_timeout() {
        let toml_str = r#"
            timeout = "10s"
        "#;
        let config: HttpConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, Duration::from_secs(10));
    }

    #[test]
    fn deserialize_http_config_timeout_minutes() {
        let toml_str = r#"
            timeout = "2m"
        "#;
        let config: HttpConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, Duration::from_secs(120));
    }

    #[test]
    fn deserialize_http_config_timeout_integer() {
        let toml_str = r"
            timeout = 45
        ";
        let config: HttpConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, Duration::from_secs(45));
    }

    #[test]
    fn deserialize_http_config_defaults() {
        let toml_str = "";
        let config: HttpConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(config.timeout, DEFAULT_HTTP_TIMEOUT);
    }

    #[test]
    fn deserialize_app_config_with_http_section() {
        let toml_str = r#"
            [auth]
            access_key = "ak"
            secret_key = "sk"

            [http]
            timeout = "60s"
        "#;

        let config: AppConfig = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(
            config.auth.credential_status(),
            CredentialStatus::BothPresent
        );
        assert_eq!(config.http.timeout, Duration::from_secs(60));
    }
}
