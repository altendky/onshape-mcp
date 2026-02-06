//! Configuration types and validation logic.
//!
//! Pure data types and validation for application configuration.
//! No I/O — config loading is handled by `onshape-mcp-io`.

use std::time::Duration;

use secrecy::SecretString;
use serde::Deserialize;

/// Default interval for periodic credential validation checks.
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes

// ============================================================================
// Configuration Types
// ============================================================================

/// Authentication configuration.
///
/// Contains optional credentials and check interval settings.
/// Credentials are wrapped in [`SecretString`] to prevent accidental logging.
#[derive(Deserialize)]
pub struct AuthConfig {
    /// Onshape API access key.
    #[serde(default)]
    pub access_key: Option<SecretString>,
    /// Onshape API secret key.
    #[serde(default)]
    pub secret_key: Option<SecretString>,
    /// Interval for periodic credential validation (default: 5 minutes).
    #[serde(
        default = "default_check_interval",
        deserialize_with = "deserialize_duration"
    )]
    pub check_interval: Duration,
}

/// Top-level application configuration.
#[derive(Default, Deserialize)]
pub struct AppConfig {
    /// Authentication settings.
    #[serde(default)]
    pub auth: AuthConfig,
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
    /// Both access key and secret key are provided.
    BothPresent,
    /// No credentials are configured.
    NonePresent,
    /// Only one credential is provided — the other is missing.
    Partial {
        /// Name of the missing credential field.
        missing: &'static str,
    },
}

impl AuthConfig {
    /// Checks whether credentials are present (without validating them).
    #[must_use]
    pub const fn credential_status(&self) -> CredentialStatus {
        match (&self.access_key, &self.secret_key) {
            (Some(_), Some(_)) => CredentialStatus::BothPresent,
            (None, None) => CredentialStatus::NonePresent,
            (Some(_), None) => CredentialStatus::Partial {
                missing: "secret_key",
            },
            (None, Some(_)) => CredentialStatus::Partial {
                missing: "access_key",
            },
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            access_key: None,
            secret_key: None,
            check_interval: DEFAULT_CHECK_INTERVAL,
        }
    }
}

// ============================================================================
// Serde Helpers
// ============================================================================

/// Default check interval for serde deserialization.
const fn default_check_interval() -> Duration {
    DEFAULT_CHECK_INTERVAL
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

    Ok(Duration::from_secs(num * multiplier))
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
            check_interval: DEFAULT_CHECK_INTERVAL,
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
            check_interval: DEFAULT_CHECK_INTERVAL,
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
            check_interval: DEFAULT_CHECK_INTERVAL,
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
        assert_eq!(config.check_interval, Duration::from_secs(300));
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
}
