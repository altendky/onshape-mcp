//! OAuth token file I/O operations.
//!
//! Handles reading and writing OAuth token files with proper permission checks.

use std::path::Path;

use onshape_client_core::oauth::OAuthTokenData;

use crate::config::{ConfigLoadError, check_file_permissions};

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
pub fn load_token_file(path: &Path) -> Result<OAuthTokenData, TokenFileError> {
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
pub fn save_token_file(path: &Path, tokens: &OAuthTokenData) -> Result<(), TokenFileError> {
    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| TokenFileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }

    let content =
        serde_json::to_string_pretty(tokens).map_err(|source| TokenFileError::Serialize {
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

    fn test_tokens() -> OAuthTokenData {
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

        let tokens = test_tokens();
        save_token_file(&path, &tokens).expect("should save");
        let loaded = load_token_file(&path).expect("should load");

        assert_eq!(loaded.access_token.secret(), "test-access-token");
        assert_eq!(loaded.refresh_token.secret(), "test-refresh-token");
        assert_eq!(loaded.token_type, "bearer");
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("nested").join("dir").join("tokens.json");

        save_token_file(&path, &test_tokens()).expect("should save");
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        save_token_file(&path, &test_tokens()).expect("should save");

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
        save_token_file(&path, &test_tokens()).expect("should save");
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
