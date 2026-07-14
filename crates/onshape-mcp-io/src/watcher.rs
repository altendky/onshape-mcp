//! Token file watcher for detecting OAuth token changes.
//!
//! Watches the token file's parent directory for changes using OS-native
//! file watching (inotify on Linux, kqueue on macOS, `ReadDirectoryChanges`
//! on Windows) with a polling fallback if the native watcher fails.
//!
//! When changes are detected, the watcher re-reads the token file and
//! updates the server's `ApiState`:
//!
//! - `NotConfigured → OAuth` when a token file with client credentials appears.
//! - `OAuthPending → OAuth` when tokens appear (client creds already known).
//! - `OAuth → OAuth` when tokens are refreshed externally.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, Watcher};
use oauth2::AccessToken;
use secrecy::ExposeSecret;

use onshape_client_core::oauth::{OAuthSession, onshape_oauth_client};
use onshape_client_io::{ClientAuthConfig, ClientConfig, OnshapeClient};

use onshape_mcp_core::ValidationState;

use crate::oauth::McpOAuthTokenFile;
use crate::{ApiState, OAuthApiState, OAuthPendingState, REFRESH_MARGIN_SECS, TokenFileSnapshot};

/// Debounce interval for file change events.
///
/// Multiple events may fire for a single file write (create + modify + close).
/// We wait this long after the last event before acting.
const DEBOUNCE_DURATION: Duration = Duration::from_millis(500);

/// Polling interval for the fallback polling watcher.
///
/// Only used if the native file watcher fails to initialize.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Context needed by the watcher to build OAuth states from scratch.
///
/// Required for the `NotConfigured → OAuth` transition when a token file
/// (containing embedded client credentials) appears after the server starts.
pub(crate) struct WatcherContext {
    /// Path to the token file to watch.
    pub token_path: PathBuf,
    /// Base URL for the Onshape API (from the `OpenAPI` spec).
    pub base_url: String,
    /// HTTP request timeout.
    pub timeout: Duration,
}

/// Spawn a background task that watches the token file for changes.
///
/// The watcher monitors the token file's parent directory (since the file
/// may not exist yet). When changes to the token file are detected:
///
/// - If the state is `NotConfigured`, attempts to transition to `OAuth`
///   using client credentials embedded in the token file.
/// - If the state is `OAuthPending`, attempts to transition to `OAuth`.
/// - If the state is `OAuth`, attempts to update tokens if fresher.
///
/// Returns a `JoinHandle` that runs until the server shuts down.
///
/// # Panics
///
/// Does not panic. If the watcher cannot be initialized, the task logs
/// the error and exits gracefully.
pub(crate) fn spawn_token_watcher(
    ctx: WatcherContext,
    api_state: Arc<tokio::sync::Mutex<ApiState>>,
    validation: Arc<tokio::sync::Mutex<ValidationState>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if run_watcher(ctx, api_state, validation).await.is_err() {
            // TODO: replace eprintln! with tracing::warn! once tracing is available
            // See: https://github.com/altendky/onshape-mcp/issues/73
            eprintln!(
                "Warning: token file watcher exited — the server continues without \
                 live token detection. Users must restart the server after \
                 completing the OAuth flow.",
            );
        }
    })
}

/// Internal: run the file watcher loop.
async fn run_watcher(
    ctx: WatcherContext,
    api_state: Arc<tokio::sync::Mutex<ApiState>>,
    validation: Arc<tokio::sync::Mutex<ValidationState>>,
) -> Result<(), ()> {
    let watch_dir = ctx.token_path.parent().map(std::path::Path::to_path_buf);
    let Some(watch_dir) = watch_dir else {
        return Err(());
    };

    let file_name = ctx
        .token_path
        .file_name()
        .map(std::ffi::OsStr::to_os_string);
    let Some(file_name) = file_name else {
        return Err(());
    };

    // Create a channel for file events.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);

    // Try native watcher first, fall back to polling.
    let _watcher = create_watcher(&watch_dir, &file_name, tx)?;

    // Event loop with debouncing.
    loop {
        // Wait for the first event.
        if rx.recv().await.is_none() {
            // Channel closed — watcher was dropped.
            return Ok(());
        }

        // Debounce: drain any events that arrive within the debounce window.
        tokio::time::sleep(DEBOUNCE_DURATION).await;
        while rx.try_recv().is_ok() {}

        // Process the change.
        handle_token_change(&ctx, &api_state, &validation).await;
    }
}

/// Create a file watcher (native with polling fallback).
///
/// Returns the watcher (must be kept alive — dropping it stops watching).
fn create_watcher(
    watch_dir: &std::path::Path,
    file_name: &std::ffi::OsStr,
    tx: tokio::sync::mpsc::Sender<()>,
) -> Result<Box<dyn Watcher + Send>, ()> {
    let file_name = file_name.to_os_string();

    // Event handler: filter to our token file and send a notification.
    let make_handler = move |tx: tokio::sync::mpsc::Sender<()>| {
        let file_name = file_name.clone();
        move |result: Result<Event, notify::Error>| {
            let Ok(event) = result else {
                return;
            };

            // Only react to create/modify/remove events.
            let dominated = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            );
            if !dominated {
                return;
            }

            // Check if any affected path matches our file name.
            let is_our_file = event
                .paths
                .iter()
                .any(|p| p.file_name().is_some_and(|n| n == file_name));
            if !is_our_file {
                return;
            }

            // Send notification (non-blocking; if channel is full, skip).
            let _ = tx.try_send(());
        }
    };

    // Try native watcher first.
    let handler = make_handler(tx.clone());
    if let Ok(mut watcher) = RecommendedWatcher::new(handler, notify::Config::default())
        && watcher
            .watch(watch_dir, notify::RecursiveMode::NonRecursive)
            .is_ok()
    {
        return Ok(Box::new(watcher));
    }

    // Fallback: polling watcher.
    let handler = make_handler(tx);
    let poll_config = notify::Config::default().with_poll_interval(POLL_INTERVAL);
    let mut watcher = notify::PollWatcher::new(handler, poll_config).map_err(|_| ())?;
    watcher
        .watch(watch_dir, notify::RecursiveMode::NonRecursive)
        .map_err(|_| ())?;
    Ok(Box::new(watcher))
}

/// Handle a token file change event.
///
/// Reads the token file and updates the API state as appropriate.
/// Resets validation state to `NotValidated` on any state transition,
/// since the new credentials need to be re-validated.
async fn handle_token_change(
    ctx: &WatcherContext,
    api_state: &Arc<tokio::sync::Mutex<ApiState>>,
    validation: &Arc<tokio::sync::Mutex<ValidationState>>,
) {
    handle_token_change_before_api_lock(ctx, api_state, validation, || {}).await;
}

async fn handle_token_change_before_api_lock<F>(
    ctx: &WatcherContext,
    api_state: &Arc<tokio::sync::Mutex<ApiState>>,
    validation: &Arc<tokio::sync::Mutex<ValidationState>>,
    before_api_lock: F,
) where
    F: FnOnce(),
{
    before_api_lock();
    let mut state = api_state.lock().await;

    // Refresh uses this same ordering while its caller holds api_state. Reading
    // only after both locks are held prevents a stale snapshot from being
    // replayed after refresh publishes newer credentials.
    let _token_lock = match crate::oauth::TokenFileLock::acquire(&ctx.token_path).await {
        Ok(token_lock) => token_lock,
        Err(e) => {
            eprintln!("Warning: failed to lock OAuth token file for watcher update: {e}");
            return;
        }
    };
    let (token_file, snapshot) = match crate::oauth::load_token_file_with_snapshot(&ctx.token_path)
    {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("Warning: failed to read OAuth token file for watcher update: {e}");
            return;
        }
    };

    if matches!(&*state, ApiState::NotConfigured { .. }) {
        match build_oauth_from_token_file(ctx, token_file, snapshot) {
            Ok(oauth) => {
                *state = ApiState::OAuth(Box::new(oauth));
                *validation.lock().await = ValidationState::default();
            }
            Err(e) => {
                eprintln!("Warning: failed to adopt watched OAuth token file: {e}");
            }
        }
        return;
    }

    if let ApiState::OAuthPending(pending) = &*state {
        match build_oauth_from_pending(pending, token_file, snapshot) {
            Ok(oauth) => {
                *state = ApiState::OAuth(Box::new(oauth));
                *validation.lock().await = ValidationState::default();
            }
            Err(e) => {
                eprintln!("Warning: failed to adopt watched OAuth token file: {e}");
            }
        }
        return;
    }

    if let ApiState::OAuth(oauth) = &mut *state {
        if oauth.last_observed_token_snapshot == snapshot {
            return;
        }
        match crate::adopt_external_token_file(
            oauth,
            &token_file,
            crate::ExternalTokenAdoptionPolicy::WatcherObservedWrite,
        ) {
            Ok(true) => {
                oauth.last_observed_token_snapshot = snapshot;
                *validation.lock().await = ValidationState::default();
            }
            Ok(false) => {
                eprintln!(
                    "Warning: failed to adopt watched OAuth token file: incomplete token or refresh metadata"
                );
            }
            Err(e) => {
                eprintln!("Warning: failed to adopt externally refreshed OAuth tokens: {e}");
            }
        }
    }
}

/// Build a full `OAuthApiState` from a token file that contains embedded credentials.
///
/// Used for the `NotConfigured → OAuth` transition when the token file
/// includes either `client_id` + `client_secret` (direct mode) or
/// `proxy_url` (proxy mode).
fn build_oauth_from_token_file(
    ctx: &WatcherContext,
    token_file: McpOAuthTokenFile,
    snapshot: TokenFileSnapshot,
) -> Result<OAuthApiState, Box<dyn std::error::Error + Send + Sync>> {
    if !crate::oauth::has_complete_token_material(&token_file) {
        return Err("token file missing nonblank access or refresh token".into());
    }
    let refresh_method = crate::refresh_method_from_token_file(&token_file)?
        .ok_or("token file missing refresh metadata")?;
    let token_metadata = refresh_method.token_metadata_from_file(&token_file);
    let session = OAuthSession::new(
        token_file.tokens,
        chrono::Duration::seconds(REFRESH_MARGIN_SECS),
    );

    let client = OnshapeClient::new(ClientConfig {
        base_url: ctx.base_url.clone(),
        auth: ClientAuthConfig::Bearer {
            access_token: AccessToken::new(session.access_token().secret().clone()),
        },
        timeout: Some(ctx.timeout),
    })?;

    let refresh_http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build refresh HTTP client: {e}"))?;

    Ok(OAuthApiState {
        session,
        token_metadata,
        refresh_method,
        client,
        refresh_http,
        token_path: ctx.token_path.clone(),
        last_observed_token_snapshot: snapshot,
        base_url: ctx.base_url.clone(),
        timeout: ctx.timeout,
    })
}

/// Build a full `OAuthApiState` from a pending state and token data.
fn build_oauth_from_pending(
    pending: &OAuthPendingState,
    token_file: McpOAuthTokenFile,
    snapshot: TokenFileSnapshot,
) -> Result<OAuthApiState, Box<dyn std::error::Error + Send + Sync>> {
    if !crate::oauth::has_complete_token_material(&token_file) {
        return Err("token file missing nonblank access or refresh token".into());
    }
    let pending_refresh_method = || match &pending.refresh_method {
        crate::PendingRefreshMethod::Direct {
            client_id,
            client_secret,
        } => {
            let oauth_client = onshape_oauth_client(client_id, client_secret.expose_secret());
            crate::RefreshMethod::Direct {
                oauth_client: Box::new(oauth_client),
                client_id: client_id.clone(),
                client_secret: secrecy::SecretString::from(client_secret.clone()),
            }
        }
        crate::PendingRefreshMethod::Proxy { proxy_url } => crate::RefreshMethod::Proxy {
            proxy_url: proxy_url.clone(),
        },
    };

    // A newly written token file records the mode actually used by the login
    // flow. Prefer that complete metadata, even when it differs from startup
    // configuration, and use the pending method only for older/minimal files.
    let refresh_method =
        crate::refresh_method_from_token_file(&token_file)?.unwrap_or_else(pending_refresh_method);
    let token_metadata = match &refresh_method {
        crate::RefreshMethod::Direct {
            client_id,
            client_secret,
            ..
        } => crate::oauth::McpOAuthTokenMetadata {
            client_id: Some(client_id.clone()),
            client_secret: Some(client_secret.expose_secret().to_string()),
            proxy_url: None,
        },
        crate::RefreshMethod::Proxy { proxy_url } => crate::oauth::McpOAuthTokenMetadata {
            client_id: token_file
                .client_id
                .clone()
                .filter(|client_id| !client_id.trim().is_empty()),
            client_secret: None,
            proxy_url: Some(proxy_url.clone()),
        },
    };
    let session = OAuthSession::new(
        token_file.tokens,
        chrono::Duration::seconds(REFRESH_MARGIN_SECS),
    );

    let client = OnshapeClient::new(ClientConfig {
        base_url: pending.base_url.clone(),
        auth: ClientAuthConfig::Bearer {
            access_token: AccessToken::new(session.access_token().secret().clone()),
        },
        timeout: Some(pending.timeout),
    })?;

    let refresh_http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build refresh HTTP client: {e}"))?;

    Ok(OAuthApiState {
        session,
        token_metadata,
        refresh_method,
        client,
        refresh_http,
        token_path: pending.token_path.clone(),
        last_observed_token_snapshot: snapshot,
        base_url: pending.base_url.clone(),
        timeout: pending.timeout,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use oauth2::{AccessToken, RefreshToken};
    use secrecy::{ExposeSecret, SecretString};

    use onshape_client_core::oauth::OAuthTokenData;

    use super::*;
    use crate::{PendingRefreshMethod, RefreshMethod};

    fn token_file(
        client_id: Option<&str>,
        client_secret: Option<&str>,
        proxy_url: Option<&str>,
    ) -> McpOAuthTokenFile {
        McpOAuthTokenFile {
            tokens: OAuthTokenData {
                access_token: AccessToken::new("access".to_string()),
                refresh_token: RefreshToken::new("refresh".to_string()),
                expires_at: None,
                token_type: "bearer".to_string(),
                scopes: None,
            },
            client_id: client_id.map(str::to_string),
            client_secret: client_secret.map(str::to_string),
            proxy_url: proxy_url.map(str::to_string),
        }
    }

    fn pending(refresh_method: PendingRefreshMethod) -> OAuthPendingState {
        OAuthPendingState {
            refresh_method,
            base_url: "https://cad.onshape.com/api/v6".to_string(),
            timeout: Duration::from_secs(30),
            token_path: PathBuf::from("tokens.json"),
        }
    }

    #[test]
    fn proxy_pending_adopts_direct_file_metadata() {
        let pending = pending(PendingRefreshMethod::Proxy {
            proxy_url: "https://old-proxy.example.com".to_string(),
        });

        let oauth = build_oauth_from_pending(
            &pending,
            token_file(Some("direct-id"), Some("direct-secret"), None),
            TokenFileSnapshot::Missing,
        )
        .expect("should build OAuth state");

        let RefreshMethod::Direct {
            client_id,
            client_secret,
            ..
        } = oauth.refresh_method
        else {
            panic!("should adopt direct refresh metadata");
        };
        assert_eq!(client_id, "direct-id");
        assert_eq!(client_secret.expose_secret(), "direct-secret");
        assert!(oauth.token_metadata.proxy_url.is_none());
    }

    #[test]
    fn direct_pending_adopts_proxy_file_metadata() {
        let pending = pending(PendingRefreshMethod::Direct {
            client_id: "old-id".to_string(),
            client_secret: SecretString::from("old-secret"),
        });

        let oauth = build_oauth_from_pending(
            &pending,
            token_file(
                Some("proxy-client"),
                None,
                Some("https://new-proxy.example.com"),
            ),
            TokenFileSnapshot::Missing,
        )
        .expect("should build OAuth state");

        let RefreshMethod::Proxy { proxy_url } = oauth.refresh_method else {
            panic!("should adopt proxy refresh metadata");
        };
        assert_eq!(proxy_url, "https://new-proxy.example.com");
        assert_eq!(
            oauth.token_metadata.proxy_url.as_deref(),
            Some("https://new-proxy.example.com")
        );
        assert!(oauth.token_metadata.client_secret.is_none());
    }

    #[test]
    fn missing_file_metadata_falls_back_to_pending_method() {
        let pending = pending(PendingRefreshMethod::Direct {
            client_id: "pending-id".to_string(),
            client_secret: SecretString::from("pending-secret"),
        });

        let oauth = build_oauth_from_pending(
            &pending,
            token_file(None, None, None),
            TokenFileSnapshot::Missing,
        )
        .expect("should build OAuth state");

        assert!(matches!(oauth.refresh_method, RefreshMethod::Direct { .. }));
        assert_eq!(
            oauth.token_metadata.client_id.as_deref(),
            Some("pending-id")
        );
        assert_eq!(
            oauth.token_metadata.client_secret.as_deref(),
            Some("pending-secret")
        );
    }

    #[test]
    fn invalid_file_metadata_falls_back_to_pending_method() {
        let pending = pending(PendingRefreshMethod::Proxy {
            proxy_url: "https://pending-proxy.example.com".to_string(),
        });

        let oauth = build_oauth_from_pending(
            &pending,
            token_file(Some(" "), Some(""), Some("  ")),
            TokenFileSnapshot::Missing,
        )
        .expect("should build OAuth state");

        let RefreshMethod::Proxy { proxy_url } = oauth.refresh_method else {
            panic!("should retain pending proxy method");
        };
        assert_eq!(proxy_url, "https://pending-proxy.example.com");
        assert!(oauth.token_metadata.client_id.is_none());
        assert_eq!(
            oauth.token_metadata.proxy_url.as_deref(),
            Some("https://pending-proxy.example.com")
        );
    }

    #[test]
    fn invalid_watched_proxy_metadata_is_not_adopted_from_pending() {
        let pending = pending(PendingRefreshMethod::Direct {
            client_id: "pending-id".to_string(),
            client_secret: SecretString::from("pending-secret"),
        });
        let invalid = token_file(Some("proxy-client"), None, Some("http://proxy.example.com"));

        assert!(build_oauth_from_pending(&pending, invalid, TokenFileSnapshot::Missing).is_err());
    }

    #[tokio::test]
    async fn conflicting_watched_metadata_does_not_mutate_oauth_or_validation_state() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        let ctx = WatcherContext {
            token_path: token_path.clone(),
            base_url: "https://cad.onshape.com/api/v6".to_string(),
            timeout: Duration::from_secs(30),
        };
        let mut current = token_file(Some("current-id"), Some("current-secret"), None);
        current.tokens.access_token = AccessToken::new("current-access".to_string());
        current.tokens.refresh_token = RefreshToken::new("current-refresh".to_string());
        let oauth = build_oauth_from_token_file(&ctx, current, TokenFileSnapshot::Missing)
            .expect("should build current OAuth state");
        let api_state = Arc::new(tokio::sync::Mutex::new(ApiState::OAuth(Box::new(oauth))));
        let original_validation = ValidationState {
            status: onshape_mcp_core::ValidationStatus::Valid,
            last_check: Some(chrono::Utc::now()),
            message: Some("working".to_string()),
        };
        let validation = Arc::new(tokio::sync::Mutex::new(original_validation.clone()));
        let conflicting = token_file(
            Some("conflicting-id"),
            Some("conflicting-secret"),
            Some("https://proxy.example.com"),
        );
        crate::oauth::save_token_file(&token_path, &conflicting)
            .await
            .expect("should save conflicting token file");

        handle_token_change(&ctx, &api_state, &validation).await;

        {
            let state = api_state.lock().await;
            let ApiState::OAuth(oauth) = &*state else {
                panic!("state must remain OAuth");
            };
            assert_eq!(oauth.session.access_token().secret(), "current-access");
            assert_eq!(oauth.session.refresh_token().secret(), "current-refresh");
            let RefreshMethod::Direct {
                client_id,
                client_secret,
                ..
            } = &oauth.refresh_method
            else {
                panic!("refresh method must remain direct");
            };
            assert_eq!(client_id, "current-id");
            assert_eq!(client_secret.expose_secret(), "current-secret");
            assert_eq!(
                oauth.token_metadata.client_id.as_deref(),
                Some("current-id")
            );
            assert_eq!(
                oauth.token_metadata.client_secret.as_deref(),
                Some("current-secret")
            );
            assert!(oauth.token_metadata.proxy_url.is_none());
            drop(state);
        }
        assert_eq!(*validation.lock().await, original_validation);
    }

    #[tokio::test]
    async fn watcher_records_adopted_snapshot_and_ignores_duplicate_event() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        let ctx = WatcherContext {
            token_path: token_path.clone(),
            base_url: "https://cad.onshape.com/api/v6".to_string(),
            timeout: Duration::from_secs(30),
        };
        let current = token_file(Some("current-id"), Some("current-secret"), None);
        crate::oauth::save_token_file(&token_path, &current)
            .await
            .expect("should save current tokens");
        let oauth =
            build_oauth_from_token_file(&ctx, current, TokenFileSnapshot::capture(&token_path))
                .expect("should build OAuth state");
        let api_state = Arc::new(tokio::sync::Mutex::new(ApiState::OAuth(Box::new(oauth))));
        let validation = Arc::new(tokio::sync::Mutex::new(ValidationState::default()));
        let mut replacement = token_file(Some("new-id"), Some("new-secret"), None);
        replacement.tokens.access_token = AccessToken::new("replacement-access".to_string());
        crate::oauth::save_token_file(&token_path, &replacement)
            .await
            .expect("should save replacement tokens");

        handle_token_change(&ctx, &api_state, &validation).await;
        let adopted_snapshot = TokenFileSnapshot::capture(&token_path);
        {
            let state = api_state.lock().await;
            let ApiState::OAuth(oauth) = &*state else {
                panic!("state must remain OAuth");
            };
            assert_eq!(oauth.session.access_token().secret(), "replacement-access");
            assert_eq!(oauth.last_observed_token_snapshot, adopted_snapshot);
            drop(state);
        }

        let valid = ValidationState {
            status: onshape_mcp_core::ValidationStatus::Valid,
            last_check: Some(chrono::Utc::now()),
            message: Some("still valid".to_string()),
        };
        *validation.lock().await = valid.clone();
        handle_token_change(&ctx, &api_state, &validation).await;
        assert_eq!(*validation.lock().await, valid);
    }

    #[tokio::test]
    async fn delayed_watcher_cannot_replay_snapshot_from_before_refresh_publication() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        let ctx = WatcherContext {
            token_path: token_path.clone(),
            base_url: "https://cad.onshape.com/api/v6".to_string(),
            timeout: Duration::from_secs(30),
        };
        let mut pre_refresh_tokens = token_file(Some("client-id"), Some("client-secret"), None);
        pre_refresh_tokens.tokens.access_token = AccessToken::new("stale-access".to_string());
        crate::oauth::save_token_file(&token_path, &pre_refresh_tokens)
            .await
            .expect("should save stale tokens");
        let oauth = build_oauth_from_token_file(
            &ctx,
            pre_refresh_tokens,
            TokenFileSnapshot::capture(&token_path),
        )
        .expect("should build initial OAuth state");
        let api_state = Arc::new(tokio::sync::Mutex::new(ApiState::OAuth(Box::new(oauth))));
        let original_validation = ValidationState {
            status: onshape_mcp_core::ValidationStatus::Valid,
            last_check: Some(chrono::Utc::now()),
            message: Some("refreshed credentials are valid".to_string()),
        };
        let validation = Arc::new(tokio::sync::Mutex::new(original_validation.clone()));

        let mut api_guard = api_state.lock().await;
        let (watcher_started_tx, watcher_started_rx) = tokio::sync::oneshot::channel();
        let watcher_ctx = WatcherContext {
            token_path: ctx.token_path.clone(),
            base_url: ctx.base_url.clone(),
            timeout: ctx.timeout,
        };
        let watcher_api_state = Arc::clone(&api_state);
        let watcher_validation = Arc::clone(&validation);
        let watcher = tokio::spawn(async move {
            handle_token_change_before_api_lock(
                &watcher_ctx,
                &watcher_api_state,
                &watcher_validation,
                || {
                    watcher_started_tx
                        .send(())
                        .expect("refresh should wait for watcher start");
                },
            )
            .await;
        });
        watcher_started_rx
            .await
            .expect("watcher should reach the API-state lock");

        let token_lock = crate::oauth::TokenFileLock::acquire(&token_path)
            .await
            .expect("refresh should acquire token lock while holding API state");
        let mut refreshed = token_file(Some("client-id"), Some("client-secret"), None);
        refreshed.tokens.access_token = AccessToken::new("refreshed-access".to_string());
        crate::oauth::save_token_file_locked(&token_path, &refreshed, &token_lock)
            .expect("refresh should publish tokens");
        let refreshed_snapshot = TokenFileSnapshot::capture(&token_path);
        let ApiState::OAuth(oauth) = &mut *api_guard else {
            panic!("state must remain OAuth");
        };
        assert!(
            crate::adopt_external_token_file(
                oauth,
                &refreshed,
                crate::ExternalTokenAdoptionPolicy::WatcherObservedWrite,
            )
            .expect("refresh publication should be valid")
        );
        oauth.last_observed_token_snapshot = refreshed_snapshot.clone();
        drop(token_lock);
        drop(api_guard);

        watcher.await.expect("watcher should finish");
        {
            let api_guard = api_state.lock().await;
            let ApiState::OAuth(oauth) = &*api_guard else {
                panic!("state must remain OAuth");
            };
            assert_eq!(oauth.session.access_token().secret(), "refreshed-access");
            assert_eq!(oauth.last_observed_token_snapshot, refreshed_snapshot);
            drop(api_guard);
        }
        assert_eq!(*validation.lock().await, original_validation);
    }

    #[test]
    fn empty_token_material_cannot_transition_pending_or_unconfigured_state() {
        let pending = pending(PendingRefreshMethod::Direct {
            client_id: "pending-id".to_string(),
            client_secret: SecretString::from("pending-secret"),
        });
        let mut invalid = token_file(Some("direct-id"), Some("direct-secret"), None);
        invalid.tokens.refresh_token = RefreshToken::new(String::new());
        let ctx = WatcherContext {
            token_path: PathBuf::from("tokens.json"),
            base_url: "https://cad.onshape.com/api/v6".to_string(),
            timeout: Duration::from_secs(30),
        };

        assert!(
            build_oauth_from_pending(&pending, invalid.clone(), TokenFileSnapshot::Missing)
                .is_err()
        );
        assert!(build_oauth_from_token_file(&ctx, invalid, TokenFileSnapshot::Missing).is_err());
    }
}
