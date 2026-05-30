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
use crate::{ApiState, OAuthApiState, OAuthPendingState, REFRESH_MARGIN_SECS};

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
    // Try to load the token file. If it fails (e.g., partial write, deleted),
    // just ignore and wait for the next event.
    let Ok(token_file) = crate::oauth::load_token_file(&ctx.token_path) else {
        return;
    };

    let mut state = api_state.lock().await;

    // Case 1: Transition from NotConfigured → OAuth.
    //
    // The token file may include embedded client credentials (written by the
    // OpenCode plugin). If we're not configured and the file has credentials,
    // transition directly to OAuth.
    let transition = if matches!(&*state, ApiState::NotConfigured { .. }) {
        build_oauth_from_token_file(ctx, token_file.clone()).ok()
    } else {
        None
    };

    if let Some(oauth) = transition {
        *state = ApiState::OAuth(Box::new(oauth));
        // Reset validation — new credentials need re-validation.
        *validation.lock().await = ValidationState::default();
        return;
    }

    // Case 2: Transition from OAuthPending → OAuth.
    let transition = if let ApiState::OAuthPending(pending) = &*state {
        build_oauth_from_pending(pending, token_file.clone()).ok()
    } else {
        None
    };

    if let Some(oauth) = transition {
        *state = ApiState::OAuth(Box::new(oauth));
        // Reset validation — new credentials need re-validation.
        *validation.lock().await = ValidationState::default();
        return;
    }

    // Case 3: Update existing OAuth state with fresher tokens.
    if let ApiState::OAuth(oauth) = &mut *state
        && oauth
            .session
            .apply_external_tokens(token_file.tokens.clone(), chrono::Utc::now())
    {
        oauth.token_metadata = oauth.refresh_method.token_metadata_from_file(&token_file);
        // Tokens were updated — rebuild the HTTP client.
        let _ = oauth.rebuild_client();
        // Reset validation — external token refresh means credentials changed.
        *validation.lock().await = ValidationState::default();
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
) -> Result<OAuthApiState, Box<dyn std::error::Error + Send + Sync>> {
    let refresh_method = refresh_method_from_token_file(&token_file)?;
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
        base_url: ctx.base_url.clone(),
        timeout: ctx.timeout,
    })
}

/// Determine the refresh method from token file data.
fn refresh_method_from_token_file(
    token_file: &McpOAuthTokenFile,
) -> Result<crate::RefreshMethod, Box<dyn std::error::Error + Send + Sync>> {
    // Proxy mode: token file has proxy_url.
    if let Some(proxy_url) = &token_file.proxy_url {
        return Ok(crate::RefreshMethod::Proxy {
            proxy_url: proxy_url.clone(),
        });
    }

    // Direct mode: token file has client_id + client_secret.
    let client_id = token_file
        .client_id
        .clone()
        .ok_or("token file missing client_id and proxy_url")?;
    let client_secret = token_file
        .client_secret
        .clone()
        .ok_or("token file missing client_secret and proxy_url")?;

    let oauth_client = onshape_oauth_client(&client_id, &client_secret);
    Ok(crate::RefreshMethod::Direct {
        oauth_client: Box::new(oauth_client),
        client_id,
        client_secret: secrecy::SecretString::from(client_secret),
    })
}

/// Build a full `OAuthApiState` from a pending state and token data.
fn build_oauth_from_pending(
    pending: &OAuthPendingState,
    token_file: McpOAuthTokenFile,
) -> Result<OAuthApiState, Box<dyn std::error::Error + Send + Sync>> {
    let refresh_method = match &pending.refresh_method {
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

    let token_metadata = refresh_method.token_metadata_from_file(&token_file);
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
        base_url: pending.base_url.clone(),
        timeout: pending.timeout,
    })
}
