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
    /// Config file has permissions that are too open.
    #[error(
        "Config file {path} has insecure permissions (mode {mode:04o}). \
         Expected 0600 (owner read/write only). \
         Fix with: chmod 600 {path}"
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
/// On Unix, the file must have mode `0600` (owner read/write only).
/// On other platforms, this is a no-op that always succeeds.
///
/// # Errors
///
/// Returns `ConfigLoadError::InsecurePermissions` if the file permissions are too open.
/// Returns `ConfigLoadError::MetadataError` if file metadata cannot be read.
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
        if mode != 0o600 {
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
    let config_path = config_path_override
        .map(Path::to_path_buf)
        .or_else(default_config_path);

    // Start with hardcoded defaults
    let mut figment = Figment::from(Toml::string(DEFAULTS_TOML));

    // Layer in config file if it exists
    if let Some(ref path) = config_path
        && path.exists()
    {
        check_file_permissions(path)?;
        figment = figment.merge(Toml::file(path));
    }

    // Layer in environment variables
    // ONSHAPE_MCP_AUTH__ACCESS_KEY -> auth.access_key (double underscore = nesting)
    figment = figment.merge(Env::prefixed("ONSHAPE_MCP_").split("__"));

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
