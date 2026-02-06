//! Configuration loading with I/O.
//!
//! Loads [`AppConfig`] from layered sources using `figment`:
//! defaults → config file (TOML) → environment variables.
//!
//! CLI flags are merged by the binary crate at a higher priority.

use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use onshape_mcp_core::config::AppConfig;

/// The environment variable prefix used for all configuration.
///
/// Environment variables matching `ONSHAPE_MCP_*` are loaded by `figment`
/// with double-underscore (`__`) as the nesting separator.
/// For example, `ONSHAPE_MCP_AUTH__ACCESS_KEY` maps to `auth.access_key`.
pub const ENV_PREFIX: &str = "ONSHAPE_MCP_";

// ============================================================================
// Config File Path
// ============================================================================

/// Returns the default config file path for the current platform.
///
/// - **Unix:** `~/.config/onshape-mcp/config.toml`
/// - **Windows:** `%APPDATA%\onshape-mcp\config.toml`
///
/// Returns `None` if the platform config directory cannot be determined.
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("onshape-mcp").join("config.toml"))
}

// ============================================================================
// File Permission Checks
// ============================================================================

/// Errors that can occur during configuration loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    /// Explicitly specified config file was not found.
    #[error("Config file not found: {path}")]
    ConfigFileNotFound {
        /// Path to the missing config file.
        path: String,
    },

    /// Config file has permissions that are too open.
    #[error(
        "Config file {path} has insecure permissions (mode {mode:04o}). \
         Group and other permissions must be unset (no access for non-owner). \
         Fix with: chmod go= {path}"
    )]
    InsecurePermissions {
        /// Path to the config file.
        path: String,
        /// The actual file mode.
        mode: u32,
    },

    /// Failed to read file metadata.
    #[error("Failed to read metadata for config file {path}: {source}")]
    MetadataError {
        /// Path to the config file.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Configuration parsing or merging error.
    #[error("Configuration error: {0}")]
    Figment(Box<figment::Error>),
}

impl From<figment::Error> for ConfigLoadError {
    fn from(err: figment::Error) -> Self {
        Self::Figment(Box::new(err))
    }
}

/// Checks that a config file has secure permissions (Unix only).
///
/// On Unix, the file must not grant any group or other permissions
/// (i.e., `mode & 0o077 == 0`). Owner permissions may be any combination.
/// On other platforms, this is a no-op that always succeeds.
///
/// # Errors
///
/// Returns `ConfigLoadError::InsecurePermissions` if the file permissions are too open.
/// Returns `ConfigLoadError::MetadataError` if file metadata cannot be read.
#[cfg_attr(not(unix), expect(clippy::missing-const-for-fn))]
pub fn check_file_permissions(path: &Path) -> Result<(), ConfigLoadError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata =
            std::fs::metadata(path).map_err(|source| ConfigLoadError::MetadataError {
                path: path.display().to_string(),
                source,
            })?;

        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(ConfigLoadError::InsecurePermissions {
                path: path.display().to_string(),
                mode,
            });
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path; // suppress unused warning
    }

    Ok(())
}

// ============================================================================
// Config Loading
// ============================================================================

/// Hardcoded defaults as a TOML string.
///
/// This avoids needing `Serialize` on `AppConfig` (which contains `SecretString`
/// that intentionally doesn't implement `Serialize`).
const DEFAULTS_TOML: &str = r"
[auth]
check_interval = 300
";

/// Builds the base figment with defaults, optional config file, and env vars.
fn base_figment(config_path_override: Option<&Path>) -> Result<Figment, ConfigLoadError> {
    // Start with hardcoded defaults
    let mut figment = Figment::from(Toml::string(DEFAULTS_TOML));

    // Layer in config file
    if let Some(path) = config_path_override {
        // Explicit override: file MUST exist
        if !path.exists() {
            return Err(ConfigLoadError::ConfigFileNotFound {
                path: path.display().to_string(),
            });
        }
        check_file_permissions(path)?;
        figment = figment.merge(Toml::file(path));
    } else if let Some(ref path) = default_config_path()
        && path.exists()
    {
        // Default path: silently skip if not present
        check_file_permissions(path)?;
        figment = figment.merge(Toml::file(path));
    }

    // Layer in environment variables
    // ONSHAPE_MCP_AUTH__ACCESS_KEY -> auth.access_key (double underscore = nesting)
    figment = figment.merge(Env::prefixed(ENV_PREFIX).split("__"));

    Ok(figment)
}

/// Loads application configuration from layered sources.
///
/// **Precedence** (lowest to highest):
/// 1. Hardcoded defaults
/// 2. Config file (TOML) — if it exists and has secure permissions
/// 3. Environment variables (`ONSHAPE_MCP_` prefix, double underscore for nesting)
///
/// # Arguments
///
/// * `config_path_override` - Override the default config file path.
///   If `None`, uses [`default_config_path()`].
///
/// # Errors
///
/// Returns an error if the config file has insecure permissions or
/// if configuration parsing fails.
pub fn load_config(config_path_override: Option<&Path>) -> Result<AppConfig, ConfigLoadError> {
    let figment = base_figment(config_path_override)?;
    let config: AppConfig = figment.extract()?;
    Ok(config)
}

/// Loads configuration and merges in CLI-provided overrides.
///
/// This is the main entry point for loading configuration in the binary crate.
/// CLI overrides take the highest priority.
///
/// # Arguments
///
/// * `config_path_override` - Override the default config file path.
/// * `cli_overrides` - Key-value pairs from CLI flags (e.g., `auth.access_key`).
///
/// # Errors
///
/// Returns an error if configuration loading or parsing fails.
pub fn load_config_with_overrides(
    config_path_override: Option<&Path>,
    cli_overrides: figment::value::Dict,
) -> Result<AppConfig, ConfigLoadError> {
    let mut figment = base_figment(config_path_override)?;

    // Layer in CLI overrides (highest priority)
    if !cli_overrides.is_empty() {
        figment = figment.merge(Serialized::defaults(cli_overrides));
    }

    let config: AppConfig = figment.extract()?;
    Ok(config)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use onshape_mcp_core::config::AppConfig;

    use super::*;

    #[test]
    fn defaults_toml_deserializes_into_app_config() {
        let config: AppConfig =
            toml::from_str(DEFAULTS_TOML).expect("DEFAULTS_TOML should deserialize into AppConfig");
        assert_eq!(
            config.auth.check_interval,
            Duration::from_secs(300),
            "DEFAULTS_TOML should set check_interval to 300s"
        );
    }

    #[test]
    fn explicit_config_path_missing_returns_error() {
        let path = Path::new("/tmp/onshape-mcp-nonexistent-config-8f3a2b.toml");
        assert!(!path.exists(), "test precondition: file must not exist");

        match load_config(Some(path)) {
            Err(ConfigLoadError::ConfigFileNotFound { path: p }) => {
                assert!(
                    p.contains("onshape-mcp-nonexistent-config"),
                    "error path should contain the file name, got: {p}",
                );
            }
            Err(other) => panic!("expected ConfigFileNotFound, got: {other:?}"),
            Ok(_) => panic!("expected error for nonexistent explicit config path"),
        }
    }

    #[test]
    fn no_config_path_uses_defaults_without_error() {
        // When no explicit path is provided and the default config file
        // doesn't exist, loading should succeed with defaults.
        if let Err(err) = load_config(None) {
            panic!("loading config with no explicit path should succeed, got: {err:?}");
        }
    }
}
