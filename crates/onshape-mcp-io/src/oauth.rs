//! OAuth token file I/O operations.
//!
//! Handles reading and writing OAuth token files with proper permission checks.

use std::path::{Path, PathBuf};

use onshape_client_core::oauth::OAuthTokenData;
use serde::{Deserialize, Serialize};

use crate::config::{ConfigLoadError, check_file_permissions};

/// Returns the default data directory for onshape-mcp on the current platform.
///
/// - **Unix:** `~/.local/share/onshape-mcp/`
/// - **macOS:** `~/Library/Application Support/onshape-mcp/`
/// - **Windows:** `%LOCALAPPDATA%\onshape-mcp\`
///
/// Returns `None` if the platform data directory cannot be determined.
#[must_use]
pub fn default_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("onshape-mcp"))
}

/// Returns the default token file path for the current platform.
///
/// - **Unix:** `~/.local/share/onshape-mcp/tokens.json`
/// - **macOS:** `~/Library/Application Support/onshape-mcp/tokens.json`
/// - **Windows:** `%LOCALAPPDATA%\onshape-mcp\tokens.json`
///
/// Returns `None` if the platform data directory cannot be determined.
#[must_use]
pub fn default_token_file_path() -> Option<PathBuf> {
    default_data_dir().map(|dir| dir.join("tokens.json"))
}

/// MCP-owned OAuth token-file metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct McpOAuthTokenMetadata {
    /// OAuth client ID used to obtain these tokens.
    pub(crate) client_id: Option<String>,
    /// OAuth client secret used for direct refresh.
    pub(crate) client_secret: Option<String>,
    /// OAuth token exchange proxy URL used for proxy refresh.
    pub(crate) proxy_url: Option<String>,
}

impl McpOAuthTokenMetadata {
    /// Combines this metadata with fresh token material for persistence.
    #[must_use]
    pub(crate) fn with_tokens(&self, tokens: OAuthTokenData) -> McpOAuthTokenFile {
        McpOAuthTokenFile {
            tokens,
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            proxy_url: self.proxy_url.clone(),
        }
    }
}

/// MCP OAuth token-file shape.
///
/// The token material is flattened so serialization stays compatible with the
/// existing persisted `tokens.json` shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct McpOAuthTokenFile {
    /// Common OAuth token material.
    #[serde(flatten)]
    pub(crate) tokens: OAuthTokenData,
    /// OAuth client ID used to obtain these tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) client_id: Option<String>,
    /// OAuth client secret used for direct refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) client_secret: Option<String>,
    /// OAuth token exchange proxy URL used for proxy refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) proxy_url: Option<String>,
}

/// Loads OAuth token data from a JSON file.
///
/// Reads the file, checks permissions (Unix only), and deserializes the JSON content.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The file has insecure permissions (Unix only)
/// - The JSON content cannot be parsed
pub(crate) fn load_token_file(path: &Path) -> Result<McpOAuthTokenFile, TokenFileError> {
    check_file_permissions(path).map_err(|e| match e {
        ConfigLoadError::InsecurePermissions { path, mode } => {
            TokenFileError::InsecurePermissions { path, mode }
        }
        ConfigLoadError::MetadataError { path, source } => {
            TokenFileError::MetadataError { path, source }
        }
        other => TokenFileError::Read {
            path: path.display().to_string(),
            source: std::io::Error::other(other.to_string()),
        },
    })?;

    let content = std::fs::read_to_string(path).map_err(|source| TokenFileError::Read {
        path: path.display().to_string(),
        source,
    })?;

    serde_json::from_str(&content).map_err(|source| TokenFileError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// Saves OAuth token data to a JSON file.
///
/// Creates parent directories if needed, writes the file, and sets
/// secure permissions (0600 on Unix).
///
/// # Errors
///
/// Returns an error if the file cannot be written or permissions cannot be set.
pub(crate) fn save_token_file(
    path: &Path,
    token_file: &McpOAuthTokenFile,
) -> Result<(), TokenFileError> {
    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| TokenFileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }

    let content =
        serde_json::to_string_pretty(token_file).map_err(|source| TokenFileError::Serialize {
            path: path.display().to_string(),
            source,
        })?;

    std::fs::write(path, content).map_err(|source| TokenFileError::Write {
        path: path.display().to_string(),
        source,
    })?;

    // Set secure permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms).map_err(|source| TokenFileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }

    Ok(())
}

/// Errors that can occur during token file operations.
#[derive(Debug, thiserror::Error)]
pub enum TokenFileError {
    /// Failed to read the token file.
    #[error("Failed to read token file {path}: {source}")]
    Read {
        /// Path to the token file.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to write the token file.
    #[error("Failed to write token file {path}: {source}")]
    Write {
        /// Path to the token file.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to parse the token file JSON.
    #[error("Failed to parse token file {path}: {source}")]
    Parse {
        /// Path to the token file.
        path: String,
        /// The underlying JSON parse error.
        source: serde_json::Error,
    },

    /// Failed to serialize token data.
    #[error("Failed to serialize token data for {path}: {source}")]
    Serialize {
        /// Path to the token file.
        path: String,
        /// The underlying JSON serialize error.
        source: serde_json::Error,
    },

    /// Token file has insecure permissions.
    #[error(
        "Token file {path} has insecure permissions (mode {mode:04o}). \
         Group and other permissions must be unset (no access for non-owner). \
         Fix with: chmod go= {path}"
    )]
    InsecurePermissions {
        /// Path to the token file.
        path: String,
        /// The actual file mode.
        mode: u32,
    },

    /// Failed to read file metadata.
    #[error("Failed to read metadata for token file {path}: {source}")]
    MetadataError {
        /// Path to the token file.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use oauth2::{AccessToken, RefreshToken};
    use tempfile::TempDir;

    use super::*;

    fn test_token_file() -> McpOAuthTokenFile {
        McpOAuthTokenFile {
            tokens: OAuthTokenData {
                access_token: AccessToken::new("test-access-token".to_string()),
                refresh_token: RefreshToken::new("test-refresh-token".to_string()),
                expires_at: None,
                token_type: "bearer".into(),
                scopes: None,
            },
            client_id: None,
            client_secret: None,
            proxy_url: None,
        }
    }

    fn test_token_material() -> OAuthTokenData {
        OAuthTokenData {
            access_token: AccessToken::new("test-access-token".to_string()),
            refresh_token: RefreshToken::new("test-refresh-token".to_string()),
            expires_at: None,
            token_type: "bearer".into(),
            scopes: None,
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        let token_file = test_token_file();
        save_token_file(&path, &token_file).expect("should save");
        let loaded = load_token_file(&path).expect("should load");

        assert_eq!(loaded.tokens.access_token.secret(), "test-access-token");
        assert_eq!(loaded.tokens.refresh_token.secret(), "test-refresh-token");
        assert_eq!(loaded.tokens.token_type, "bearer");
    }

    #[test]
    fn direct_token_file_serializes_as_flat_json() {
        let token_file = McpOAuthTokenFile {
            tokens: test_token_material(),
            client_id: Some("client-id".to_string()),
            client_secret: Some("client-secret".to_string()),
            proxy_url: None,
        };

        let value = serde_json::to_value(&token_file).expect("should serialize");

        assert_eq!(value["access_token"], "test-access-token");
        assert_eq!(value["refresh_token"], "test-refresh-token");
        assert_eq!(value["token_type"], "bearer");
        assert_eq!(value["client_id"], "client-id");
        assert_eq!(value["client_secret"], "client-secret");
        assert!(value.get("tokens").is_none());
        assert!(value.get("proxy_url").is_none());
    }

    #[test]
    fn proxy_token_file_fixture_preserves_flat_json_shape() {
        let json = r#"{
            "access_token": "proxy-access",
            "refresh_token": "proxy-refresh",
            "token_type": "bearer",
            "client_id": "proxy-client-id",
            "proxy_url": "https://proxy.example.com"
        }"#;

        let token_file: McpOAuthTokenFile = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(token_file.tokens.access_token.secret(), "proxy-access");
        assert_eq!(token_file.tokens.refresh_token.secret(), "proxy-refresh");
        assert_eq!(token_file.client_id.as_deref(), Some("proxy-client-id"));
        assert_eq!(
            token_file.proxy_url.as_deref(),
            Some("https://proxy.example.com")
        );
        assert!(token_file.client_secret.is_none());

        let value = serde_json::to_value(&token_file).expect("should serialize");
        assert_eq!(value["access_token"], "proxy-access");
        assert_eq!(value["client_id"], "proxy-client-id");
        assert_eq!(value["proxy_url"], "https://proxy.example.com");
        assert!(value.get("tokens").is_none());
        assert!(value.get("client_secret").is_none());
    }

    #[test]
    fn metadata_combines_with_fresh_token_material() {
        let metadata = McpOAuthTokenMetadata {
            client_id: Some("client-id".to_string()),
            client_secret: None,
            proxy_url: Some("https://proxy.example.com".to_string()),
        };

        let token_file = metadata.with_tokens(test_token_material());

        assert_eq!(token_file.tokens.access_token.secret(), "test-access-token");
        assert_eq!(token_file.client_id.as_deref(), Some("client-id"));
        assert_eq!(
            token_file.proxy_url.as_deref(),
            Some("https://proxy.example.com")
        );
        assert!(token_file.client_secret.is_none());
    }

    #[test]
    fn default_token_file_path_returns_mcp_path_when_available() {
        let path = default_token_file_path();
        if let Some(ref p) = path {
            assert!(p.ends_with("onshape-mcp/tokens.json"));
        }
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("nested").join("dir").join("tokens.json");

        save_token_file(&path, &test_token_file()).expect("should save");
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        save_token_file(&path, &test_token_file()).expect("should save");

        let metadata = std::fs::metadata(&path).expect("should read metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file should have 0600 permissions");
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_insecure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        // Write token file then set insecure permissions
        save_token_file(&path, &test_token_file()).expect("should save");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("should set permissions");

        match load_token_file(&path) {
            Err(TokenFileError::InsecurePermissions { .. }) => {} // expected
            Err(other) => panic!("expected InsecurePermissions, got: {other:?}"),
            Ok(_) => panic!("expected error for insecure permissions"),
        }
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let path = std::path::Path::new("/tmp/onshape-mcp-nonexistent-tokens-abc123.json");
        assert!(!path.exists());

        match load_token_file(path) {
            Err(TokenFileError::Read { .. } | TokenFileError::MetadataError { .. }) => {} // expected
            Err(other) => panic!("expected Read or MetadataError, got: {other:?}"),
            Ok(_) => panic!("expected error for nonexistent file"),
        }
    }

    #[test]
    fn load_invalid_json_returns_parse_error() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        std::fs::write(&path, "not valid json").expect("should write");

        // Set secure permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("should set permissions");
        }

        match load_token_file(&path) {
            Err(TokenFileError::Parse { .. }) => {} // expected
            Err(other) => panic!("expected Parse error, got: {other:?}"),
            Ok(_) => panic!("expected error for invalid JSON"),
        }
    }
}
