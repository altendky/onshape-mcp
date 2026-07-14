//! OAuth token file I/O operations.
//!
//! Handles reading and writing OAuth token files with proper permission checks.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use onshape_client_core::oauth::OAuthTokenData;
use serde::{Deserialize, Serialize};

use crate::config::{ConfigLoadError, check_file_permissions};

const TOKEN_LOCK_SUFFIX: &str = ".lock";
const TOKEN_LOCK_RETRY: Duration = Duration::from_millis(25);
const TOKEN_LOCK_TIMEOUT: Duration = Duration::from_secs(75);

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
    load_token_file_with_snapshot(path).map(|(token_file, _)| token_file)
}

/// Loads and snapshots one token-file publication from the same open handle.
pub(crate) fn load_token_file_with_snapshot(
    path: &Path,
) -> Result<(McpOAuthTokenFile, crate::TokenFileSnapshot), TokenFileError> {
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

    let mut file = std::fs::File::open(path).map_err(|source| TokenFileError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| TokenFileError::MetadataError {
            path: path.display().to_string(),
            source,
        })?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)
        .map_err(|source| TokenFileError::Read {
            path: path.display().to_string(),
            source,
        })?;

    let token_file = serde_json::from_slice(&content).map_err(|source| TokenFileError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    let snapshot = crate::TokenFileSnapshot::from_read(content, &metadata);
    Ok((token_file, snapshot))
}

/// Saves OAuth token data to a JSON file.
///
/// Creates parent directories if needed, writes the file, and sets
/// secure permissions (0600 on Unix).
///
/// # Errors
///
/// Returns an error if the file cannot be written or permissions cannot be set.
#[cfg(test)]
pub(crate) async fn save_token_file(
    path: &Path,
    token_file: &McpOAuthTokenFile,
) -> Result<(), TokenFileError> {
    let lock = TokenFileLock::acquire(path).await?;
    save_token_file_locked(path, token_file, &lock)
}

/// Saves a token file while the caller holds its adjacent lock.
pub(crate) fn save_token_file_locked(
    path: &Path,
    token_file: &McpOAuthTokenFile,
    _lock: &TokenFileLock,
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

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let prefix = format!(
        ".{}.tmp-",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    let mut file = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)
        .map_err(|source| TokenFileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|source| TokenFileError::Write {
                path: path.display().to_string(),
                source,
            })?;
    }
    file.write_all(content.as_bytes())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|source| TokenFileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    file.persist(path).map_err(|error| TokenFileError::Write {
        path: path.display().to_string(),
        source: error.error,
    })?;

    Ok(())
}

/// Exclusive cross-process token writer lock.
///
/// Rust and TypeScript writers both exclusively create the adjacent
/// `tokens.json.lock` directory. Writers never evict it; recovery from an
/// abandoned lock is an explicit manual operation.
#[derive(Debug)]
pub(crate) struct TokenFileLock {
    path: PathBuf,
    owner_path: PathBuf,
    cleanup_path: PathBuf,
}

impl TokenFileLock {
    pub(crate) async fn acquire(token_path: &Path) -> Result<Self, TokenFileError> {
        Self::acquire_with_policy(token_path, TOKEN_LOCK_TIMEOUT, TOKEN_LOCK_RETRY).await
    }

    async fn acquire_with_policy(
        token_path: &Path,
        timeout: Duration,
        retry: Duration,
    ) -> Result<Self, TokenFileError> {
        if let Some(parent) = token_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TokenFileError::Write {
                path: token_path.display().to_string(),
                source,
            })?;
        }

        let lock_path = token_lock_path(token_path);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match std::fs::create_dir(&lock_path) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Err(source) = std::fs::set_permissions(
                            &lock_path,
                            std::fs::Permissions::from_mode(0o700),
                        ) {
                            let _ = std::fs::remove_dir(&lock_path);
                            return Err(TokenFileError::Lock {
                                path: lock_path.display().to_string(),
                                source,
                            });
                        }
                    }
                    let owner_path = lock_path.join(format!(
                        "owner-{}-{:032x}",
                        std::process::id(),
                        rand::random::<u128>()
                    ));
                    if let Err(source) = std::fs::create_dir(&owner_path) {
                        let _ = std::fs::remove_dir(&lock_path);
                        return Err(TokenFileError::Lock {
                            path: lock_path.display().to_string(),
                            source,
                        });
                    }
                    let cleanup_path = lock_path.with_extension(format!(
                        "lock.cleanup-{}-{:032x}",
                        std::process::id(),
                        rand::random::<u128>()
                    ));
                    return Ok(Self {
                        path: lock_path,
                        owner_path,
                        cleanup_path,
                    });
                }
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(TokenFileError::LockTimeout {
                            path: lock_path.display().to_string(),
                        });
                    }
                    tokio::time::sleep(retry).await;
                }
                Err(source) => {
                    return Err(TokenFileError::Lock {
                        path: lock_path.display().to_string(),
                        source,
                    });
                }
            }
        }
    }
}

impl Drop for TokenFileLock {
    fn drop(&mut self) {
        if std::fs::rename(&self.path, &self.cleanup_path).is_ok() {
            let owner_name = self.owner_path.file_name().unwrap_or_default();
            if std::fs::remove_dir(self.cleanup_path.join(owner_name)).is_ok() {
                let _ = std::fs::remove_dir(&self.cleanup_path);
            } else {
                let _ = std::fs::rename(&self.cleanup_path, &self.path);
            }
        }
    }
}

pub(crate) fn has_complete_token_material(token_file: &McpOAuthTokenFile) -> bool {
    !token_file.tokens.access_token.secret().trim().is_empty()
        && !token_file.tokens.refresh_token.secret().trim().is_empty()
}

fn token_lock_path(token_path: &Path) -> PathBuf {
    let mut lock = token_path.as_os_str().to_owned();
    lock.push(TOKEN_LOCK_SUFFIX);
    PathBuf::from(lock)
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

    /// Failed to create, inspect, or remove the writer lock.
    #[error("Failed to manage token file lock {path}: {source}")]
    Lock {
        /// Path to the adjacent lock directory.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Another writer held the token lock for the full retry period.
    #[error(
        "Timed out waiting for token file lock directory {path}. Stop all onshape-mcp/OpenCode writers, then manually remove the lock directory if no writer is running."
    )]
    LockTimeout {
        /// Path to the adjacent lock directory.
        path: String,
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

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        let token_file = test_token_file();
        save_token_file(&path, &token_file)
            .await
            .expect("should save");
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

    #[tokio::test]
    async fn saving_direct_tokens_over_proxy_file_drops_proxy_url() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        let proxy_file = McpOAuthTokenFile {
            tokens: test_token_material(),
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some("https://oauth-proxy.example.com".to_string()),
        };
        save_token_file(&path, &proxy_file)
            .await
            .expect("should save proxy file");

        let direct_file = McpOAuthTokenFile {
            tokens: test_token_material(),
            client_id: Some("direct-client".to_string()),
            client_secret: Some("direct-secret".to_string()),
            proxy_url: None,
        };
        save_token_file(&path, &direct_file)
            .await
            .expect("should overwrite with direct file");

        let loaded = load_token_file(&path).expect("should load direct file");
        assert_eq!(loaded.client_id.as_deref(), Some("direct-client"));
        assert_eq!(loaded.client_secret.as_deref(), Some("direct-secret"));
        assert!(loaded.proxy_url.is_none());
    }

    #[test]
    fn default_token_file_path_returns_mcp_path_when_available() {
        let path = default_token_file_path();
        if let Some(ref p) = path {
            assert!(p.ends_with("onshape-mcp/tokens.json"));
        }
    }

    #[tokio::test]
    async fn save_creates_parent_directories() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("nested").join("dir").join("tokens.json");

        save_token_file(&path, &test_token_file())
            .await
            .expect("should save");
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn save_sets_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        save_token_file(&path, &test_token_file())
            .await
            .expect("should save");

        let metadata = std::fs::metadata(&path).expect("should read metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file should have 0600 permissions");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn load_rejects_insecure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");

        // Write token file then set insecure permissions
        save_token_file(&path, &test_token_file())
            .await
            .expect("should save");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("should set permissions");

        match load_token_file(&path) {
            Err(TokenFileError::InsecurePermissions { .. }) => {} // expected
            Err(other) => panic!("expected InsecurePermissions, got: {other:?}"),
            Ok(_) => panic!("expected error for insecure permissions"),
        }
    }

    #[tokio::test]
    async fn lock_serializes_login_after_refresh_finalization() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        let refresh_lock = TokenFileLock::acquire(&path)
            .await
            .expect("refresh should acquire lock");
        let mut login_file = test_token_file();
        login_file.tokens.access_token = AccessToken::new("login-access".to_string());

        let login_save = save_token_file(&path, &login_file);
        tokio::pin!(login_save);
        tokio::select! {
            result = &mut login_save => panic!("login bypassed refresh lock: {result:?}"),
            () = tokio::time::sleep(Duration::from_millis(75)) => {}
        }

        let mut refresh_file = test_token_file();
        refresh_file.tokens.access_token = AccessToken::new("refreshed-access".to_string());
        save_token_file_locked(&path, &refresh_file, &refresh_lock)
            .expect("refresh should save while locked");
        drop(refresh_lock);
        login_save.await.expect("login should save after refresh");

        let loaded = load_token_file(&path).expect("should load final login");
        assert_eq!(loaded.tokens.access_token.secret(), "login-access");
        assert!(!token_lock_path(&path).exists());
    }

    #[tokio::test]
    async fn abandoned_lock_times_out_without_deletion() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        let lock_path = token_lock_path(&path);
        std::fs::create_dir(&lock_path).expect("should create abandoned lock");

        let error = TokenFileLock::acquire_with_policy(
            &path,
            Duration::from_millis(10),
            Duration::from_millis(1),
        )
        .await
        .expect_err("abandoned lock must fail closed");
        let message = error.to_string();
        assert!(message.contains(&lock_path.display().to_string()));
        assert!(message.contains("Stop all onshape-mcp/OpenCode writers"));
        assert!(message.contains("manually remove the lock directory"));
        assert!(lock_path.is_dir(), "timed out writer must not evict lock");
    }

    #[tokio::test]
    async fn lock_is_cleaned_up_when_locked_save_fails() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        std::fs::create_dir(&path).expect("token path should be unwritable as a file");
        let lock = TokenFileLock::acquire(&path)
            .await
            .expect("should acquire lock");
        assert!(save_token_file_locked(&path, &test_token_file(), &lock).is_err());
        drop(lock);
        assert!(!token_lock_path(&path).exists());
    }

    #[tokio::test]
    async fn guard_does_not_remove_replacement_lock_directory() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        let lock = TokenFileLock::acquire(&path)
            .await
            .expect("should acquire lock");
        let lock_path = token_lock_path(&path);
        std::fs::remove_dir(&lock.owner_path).expect("should remove owner marker");
        std::fs::remove_dir(&lock_path).expect("should remove original lock");
        std::fs::create_dir(&lock_path).expect("should create replacement lock");

        drop(lock);

        assert!(lock_path.is_dir(), "old guard must not remove replacement");
    }

    #[tokio::test]
    async fn failed_atomic_publication_cleans_temp() {
        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        std::fs::create_dir(&path).expect("should block destination replacement");
        let lock = TokenFileLock::acquire(&path)
            .await
            .expect("should acquire lock");
        assert!(save_token_file_locked(&path, &test_token_file(), &lock).is_err());
        drop(lock);

        let entries = std::fs::read_dir(dir.path())
            .expect("should list directory")
            .map(|entry| entry.expect("entry should be readable").file_name())
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            vec![path.file_name().expect("should have file name")]
        );
    }

    #[tokio::test]
    async fn lock_free_readers_only_observe_complete_publications() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let dir = TempDir::new().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        save_token_file(&path, &test_token_file())
            .await
            .expect("should save baseline");

        let stop = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));
        let reader_stop = Arc::clone(&stop);
        let reader_reads = Arc::clone(&reads);
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            while !reader_stop.load(Ordering::Relaxed) {
                let contents = std::fs::read_to_string(&reader_path).expect("read should succeed");
                serde_json::from_str::<McpOAuthTokenFile>(&contents)
                    .expect("reader must only observe complete JSON");
                reader_reads.fetch_add(1, Ordering::Relaxed);
            }
        });

        for index in 0..50 {
            let mut token_file = test_token_file();
            token_file.tokens.access_token = AccessToken::new(format!("access-{index}"));
            save_token_file(&path, &token_file)
                .await
                .expect("publication should succeed");
        }
        stop.store(true, Ordering::Relaxed);
        reader.join().expect("reader should not panic");
        assert!(reads.load(Ordering::Relaxed) > 0);
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
