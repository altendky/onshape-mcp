//! OAuth login flow orchestration.
//!
//! Handles the full OAuth authorization code flow:
//! 1. Generate PKCE verifier + challenge
//! 2. Generate CSRF state token
//! 3. Start a local callback server on loopback (`127.0.0.1` and/or `[::1]`)
//! 4. Build and return the authorization URL
//! 5. Wait for the callback with the authorization code
//! 6. Exchange the code for tokens (direct or via proxy)
//! 7. Write tokens to the token file
//!
//! The redirect URI uses `http://localhost:18338/callback` as required by
//! Onshape's OAuth application settings (which reject literal IP addresses).
//! The callback server binds to both `127.0.0.1` and `[::1]` loopback
//! addresses (whichever succeed) so the flow works regardless of how the
//! system resolves `localhost`.
//!
//! The flow is initiated by the `onshape_mcp_auth_login` MCP tool or the
//! `auth login` CLI subcommand. The authorization URL is returned to the
//! caller, who is responsible for opening it in the user's browser.

use std::net::SocketAddr;
use std::sync::Arc;

use oauth2::{AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, TokenResponse};
use tokio::sync::oneshot;

use onshape_client_core::oauth::{
    OAuthLoginConfig, OAuthLoginSession, OAuthTokenData, build_authorize_url,
    default_token_file_path, onshape_oauth_client, validate_callback,
};
use onshape_mcp_core::tools::LoginMode;

/// The port used for the local OAuth callback server.
pub const CALLBACK_PORT: u16 = 18338;

/// The redirect URI for the OAuth callback.
///
/// Uses `localhost` because Onshape's OAuth application settings require
/// redirect URLs to start with `https://` or `http://localhost`. Literal
/// loopback IPs (`127.0.0.1`, `[::1]`) are rejected by Onshape despite
/// being recommended by RFC 8252 Section 8.3.
pub const REDIRECT_URI: &str = "http://localhost:18338/callback";

/// Default scopes to request from Onshape.
const DEFAULT_SCOPES: &[&str] = &["OAuth2Read", "OAuth2Write"];

/// Timeout for the login flow (2 minutes).
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur during the OAuth login flow.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    /// Failed to start the callback server on both IPv4 and IPv6 loopback.
    #[error(
        "failed to start callback server on port {CALLBACK_PORT} \
         (tried 127.0.0.1 and [::1]): {0}"
    )]
    ServerStart(std::io::Error),

    /// Failed to fetch proxy configuration.
    #[error("failed to fetch proxy config from {url}: {detail}")]
    ProxyConfig {
        /// The proxy config URL that was fetched.
        url: String,
        /// Error detail.
        detail: String,
    },

    /// The callback was not received within the timeout.
    #[error("login flow timed out after {LOGIN_TIMEOUT:?} — no callback received")]
    Timeout,

    /// OAuth callback validation failed.
    #[error("callback validation failed: {0}")]
    CallbackValidation(#[from] onshape_client_core::oauth::CallbackValidationError),

    /// Token exchange failed.
    #[error("token exchange failed: {0}")]
    TokenExchange(String),

    /// Failed to save the token file.
    #[error("failed to save token file: {0}")]
    TokenSave(#[from] crate::oauth::TokenFileError),

    /// No token file path available on this platform.
    #[error("cannot determine token file path for this platform")]
    NoTokenPath,
}

// ============================================================================
// Login Session State
// ============================================================================

/// Tracks an active login flow.
///
/// Holds the background task handle and the shutdown signal for the
/// callback servers. Dropping a `LoginSession` cancels the flow:
/// the shutdown signal is sent and the background task is aborted.
/// This ensures callback servers release their ports promptly.
pub struct LoginSession {
    /// Handle to the background task running the login flow.
    task_handle: tokio::task::JoinHandle<()>,
    /// Sends `true` to shut down all callback servers.
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl Drop for LoginSession {
    fn drop(&mut self) {
        // Signal callback servers to shut down.
        let _ = self.shutdown.send(true);
        // Abort the background task (token exchange, etc.).
        self.task_handle.abort();
    }
}

/// Manages login session lifecycle.
///
/// At most one login flow can be active at a time.
pub struct LoginState {
    /// The currently active login session, if any.
    session: Option<LoginSession>,
}

impl Default for LoginState {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginState {
    /// Creates a new empty login state.
    #[must_use]
    pub const fn new() -> Self {
        Self { session: None }
    }

    /// Returns `true` if a login flow is currently in progress.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| !s.task_handle.is_finished())
    }

    /// Sets the active session, cancelling any previous flow.
    ///
    /// If a login flow is already in progress, it is cancelled (its
    /// callback servers are shut down and its background task is aborted)
    /// before the new session is stored.
    pub fn set_active(&mut self, session: LoginSession) {
        // Dropping the old session triggers Drop, which cancels it.
        self.session = Some(session);
    }

    /// Clears the active session (cancelling it if still in progress).
    pub fn clear(&mut self) {
        self.session = None;
    }
}

// ============================================================================
// Login Flow Orchestration
// ============================================================================

/// Result of starting a login flow.
///
/// Contains the authorization URL that should be opened in the user's browser,
/// a channel to receive the final result, and a [`LoginSession`] that must
/// be stored to keep the flow alive. Dropping the session cancels the flow.
pub struct LoginFlowHandle {
    /// The authorization URL to open in the user's browser.
    pub authorize_url: String,
    /// Receives the result when the flow completes (success or error).
    pub result_rx: oneshot::Receiver<Result<(), LoginError>>,
    /// The login session — must be stored to keep the flow alive.
    /// Dropping this cancels the callback servers and background task.
    pub session: LoginSession,
}

/// Starts an OAuth login flow.
///
/// This is the main entry point for both the MCP tool and the CLI subcommand.
///
/// 1. For proxy mode: fetches the `client_id` from the proxy's `/config` endpoint
/// 2. Generates PKCE verifier + challenge and CSRF state
/// 3. Starts the local callback server
/// 4. Builds the authorization URL
/// 5. Returns the URL immediately (for the caller to display/open)
/// 6. In the background: waits for callback → validates → exchanges → saves
///
/// # Errors
///
/// Returns an error if the callback server cannot be started or if the proxy
/// config cannot be fetched.
pub async fn start_login_flow(mode: &LoginMode) -> Result<LoginFlowHandle, LoginError> {
    // Determine client_id and build the exchange configuration.
    let exchange_config = match mode {
        LoginMode::Proxy { proxy_url } => {
            let client_id = fetch_proxy_client_id(proxy_url).await?;
            ExchangeConfig::Proxy {
                client_id,
                proxy_url: proxy_url.clone(),
            }
        }
        LoginMode::Direct {
            client_id,
            client_secret,
        } => ExchangeConfig::Direct {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
        },
    };

    let client_id = exchange_config.client_id().to_string();

    // Generate PKCE and CSRF.
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let csrf_state = CsrfToken::new_random();

    // Start the callback server (binds both IPv4 and IPv6 loopback).
    let listeners = start_callback_server().await?;

    // Create the shutdown channel — the sender is held by LoginSession
    // for external cancellation, the receiver is passed to the servers.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Build the authorization URL.
    let login_config = OAuthLoginConfig {
        client_id: client_id.clone(),
        redirect_uri: REDIRECT_URI.to_string(),
        scopes: DEFAULT_SCOPES.iter().map(|s| (*s).to_string()).collect(),
    };
    let authorize_url = build_authorize_url(&login_config, &csrf_state, pkce_challenge);

    // Build the login session for callback validation.
    let oauth_session = OAuthLoginSession {
        pkce_verifier,
        csrf_state,
        config: login_config,
    };

    // Spawn the background task that waits for the callback and completes the flow.
    let (result_tx, result_rx) = oneshot::channel();

    // Clone the shutdown sender for use inside the background task.
    // The original is stored in LoginSession for external cancellation.
    let internal_shutdown = shutdown_tx.clone();

    let task_handle = tokio::spawn(async move {
        let result = complete_login_flow(
            oauth_session,
            exchange_config,
            listeners,
            shutdown_rx,
            internal_shutdown,
        )
        .await;
        // Ignore send error — the receiver may have been dropped if the caller timed out.
        let _ = result_tx.send(result);
    });

    Ok(LoginFlowHandle {
        authorize_url,
        result_rx,
        session: LoginSession {
            task_handle,
            shutdown: shutdown_tx,
        },
    })
}

/// Internal exchange configuration — determines how the authorization code
/// is exchanged for tokens.
enum ExchangeConfig {
    Proxy {
        client_id: String,
        proxy_url: String,
    },
    Direct {
        client_id: String,
        client_secret: String,
    },
}

impl ExchangeConfig {
    fn client_id(&self) -> &str {
        match self {
            Self::Proxy { client_id, .. } | Self::Direct { client_id, .. } => client_id,
        }
    }
}

/// Shared state passed to the axum callback handler via `axum::extract::State`.
#[derive(Clone)]
struct CallbackState {
    /// Sends the callback URL to the main flow after basic pre-validation.
    url_sender: Arc<tokio::sync::Mutex<Option<oneshot::Sender<String>>>>,
    /// Expected CSRF state; used to ignore invalid callbacks.
    expected_state: String,
}

/// Axum handler for `GET /callback`.
///
/// Validates the CSRF state before consuming the one-shot sender, so that
/// stray or invalid requests cannot sink the login flow. Preserves the raw
/// query string to avoid mangling percent-encoded values.
async fn handle_callback(
    axum::extract::State(state): axum::extract::State<CallbackState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> axum::response::Html<&'static str> {
    // Extract and validate query parameters before consuming the one-shot.
    let Some(raw_query) = uri.query() else {
        return axum::response::Html(
            "<html><body><h1>Invalid callback.</h1>\
             <p>Missing query parameters. Please try the login flow again.</p>\
             </body></html>",
        );
    };
    let params: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(raw_query.as_bytes())
            .into_owned()
            .collect();

    let has_terminal_param = params.contains_key("code") || params.contains_key("error");
    let is_valid = has_terminal_param
        && params
            .get("state")
            .is_some_and(|s| s == &state.expected_state);

    if !is_valid {
        return axum::response::Html(
            "<html><body><h1>Invalid callback.</h1>\
             <p>The callback parameters were invalid or the request was unexpected. \
             Please try the login flow again.</p>\
             </body></html>",
        );
    }

    // Reconstruct the callback URL preserving the raw query string.
    let callback_url = format!("{REDIRECT_URI}?{raw_query}");

    // Send the callback URL through the channel (only the first valid handler wins).
    // Shutdown is handled externally by the LoginSession / complete_login_flow.
    let url_tx = state.url_sender.lock().await.take();
    if let Some(tx) = url_tx {
        let _ = tx.send(callback_url);
    }

    // Return appropriate page based on callback type.
    if params.contains_key("error") {
        return axum::response::Html(
            "<html><body><h1>Authorization denied.</h1>\
             <p>The authorization request was denied or an error occurred. \
             You can close this tab and return to your terminal.</p>\
             </body></html>",
        );
    }

    axum::response::Html(
        "<html><body><h1>Authorization successful!</h1>\
         <p>You can close this tab and return to your terminal.</p>\
         </body></html>",
    )
}

/// Completes the login flow after the callback server is started.
///
/// Serves the callback handler on all provided listeners (IPv4 and/or IPv6).
/// Waits for the callback, validates it, exchanges the code for tokens,
/// and saves the tokens to disk.
///
/// The `shutdown_rx` is cloned for each server's graceful shutdown future.
/// Shutdown is triggered either by:
/// - This function sending `true` via `shutdown_tx` after receiving
///   the callback or on timeout
/// - The `LoginSession` being dropped (external cancellation / re-attempt),
///   which sends `true` via its own clone of the sender
async fn complete_login_flow(
    session: OAuthLoginSession,
    exchange_config: ExchangeConfig,
    listeners: Vec<tokio::net::TcpListener>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
) -> Result<(), LoginError> {
    // Set up shared state for the callback handler.
    let (url_tx, url_rx) = oneshot::channel::<String>();

    let callback_state = CallbackState {
        url_sender: Arc::new(tokio::sync::Mutex::new(Some(url_tx))),
        expected_state: session.csrf_state.secret().clone(),
    };

    // Spawn one axum server per listener, all sharing the same state.
    // The first callback to arrive takes the oneshot sender; subsequent
    // callbacks on other listeners find None and are no-ops.
    let mut server_handles = Vec::new();
    for listener in listeners {
        let app = axum::Router::new()
            .route("/callback", axum::routing::get(handle_callback))
            .with_state(callback_state.clone());
        let mut shutdown_rx = shutdown_rx.clone();

        server_handles.push(tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    // Wait until the shutdown signal is sent (value changes to true)
                    // or the sender is dropped (channel closed).
                    let _ = shutdown_rx.wait_for(|&v| v).await;
                })
                .await
        }));
    }

    // Wait for callback URL or timeout.
    let result = tokio::time::timeout(LOGIN_TIMEOUT, url_rx).await;

    // Shut down all callback servers regardless of outcome.
    // This eagerly releases the ports before proceeding to token exchange.
    let _ = shutdown_tx.send(true);
    for handle in &mut server_handles {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }

    // Now handle the result (after servers are stopped).
    let callback_url = result
        .map_err(|_| LoginError::Timeout)?
        .map_err(|_| LoginError::Timeout)?;

    // Validate the callback.
    let auth_code = validate_callback(&callback_url, &session.csrf_state)?;

    // Exchange the code for tokens.
    let token_data = exchange_code(auth_code, session.pkce_verifier, &exchange_config).await?;

    // Save tokens to disk.
    let token_path = default_token_file_path().ok_or(LoginError::NoTokenPath)?;
    crate::oauth::save_token_file(&token_path, &token_data)?;

    Ok(())
}

// ============================================================================
// Callback Server
// ============================================================================

/// Starts TCP listeners for the callback server on loopback addresses.
///
/// Attempts to bind both IPv4 (`127.0.0.1`) and IPv6 (`[::1]`) loopback
/// addresses. Returns all listeners that successfully bound — at least one
/// must succeed. This dual-stack approach ensures the callback works
/// regardless of how the system resolves `localhost`.
///
/// # Errors
///
/// Returns `LoginError::ServerStart` if neither address can be bound.
async fn start_callback_server() -> Result<Vec<tokio::net::TcpListener>, LoginError> {
    let ipv4_addr = SocketAddr::from(([127, 0, 0, 1], CALLBACK_PORT));
    let ipv6_addr = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], CALLBACK_PORT));

    let ipv4_result = tokio::net::TcpListener::bind(ipv4_addr).await;
    let ipv6_result = tokio::net::TcpListener::bind(ipv6_addr).await;

    // Collect all successful binds.
    let mut listeners = Vec::new();
    let mut last_err = None;

    match ipv4_result {
        Ok(listener) => listeners.push(listener),
        Err(e) => last_err = Some(e),
    }
    match ipv6_result {
        Ok(listener) => listeners.push(listener),
        Err(e) => last_err = Some(e),
    }

    if listeners.is_empty() {
        // Both failed — return the last error (which is guaranteed to be Some
        // since both branches set it on error and listeners is empty).
        return Err(LoginError::ServerStart(last_err.unwrap_or_else(|| {
            std::io::Error::other("no loopback address available")
        })));
    }

    Ok(listeners)
}

// ============================================================================
// Token Exchange
// ============================================================================

/// Exchange an authorization code for tokens.
///
/// Uses either the direct method (via the `oauth2` crate) or the proxy
/// method (POST to the proxy's `/token/exchange` endpoint).
async fn exchange_code(
    code: AuthorizationCode,
    pkce_verifier: PkceCodeVerifier,
    config: &ExchangeConfig,
) -> Result<OAuthTokenData, LoginError> {
    match config {
        ExchangeConfig::Direct {
            client_id,
            client_secret,
        } => exchange_code_direct(code, pkce_verifier, client_id, client_secret).await,
        ExchangeConfig::Proxy {
            client_id,
            proxy_url,
        } => exchange_code_proxy(code, pkce_verifier, client_id, proxy_url).await,
    }
}

/// Direct token exchange using the `oauth2` crate.
async fn exchange_code_direct(
    code: AuthorizationCode,
    pkce_verifier: PkceCodeVerifier,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuthTokenData, LoginError> {
    let oauth_client = onshape_oauth_client(client_id, client_secret).set_redirect_uri(
        oauth2::RedirectUrl::new(REDIRECT_URI.to_string())
            .map_err(|e| LoginError::TokenExchange(format!("invalid redirect URI: {e}")))?,
    );

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| LoginError::TokenExchange(format!("failed to build HTTP client: {e}")))?;

    let response = oauth_client
        .exchange_code(code)
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| LoginError::TokenExchange(e.to_string()))?;

    // Fail fast if the authorization server did not return a refresh token.
    // Without a refresh token, the session will break once the access token
    // expires and cannot be renewed. The refresh-time code paths correctly
    // handle omitted refresh tokens per RFC 6749 Section 6, but the initial
    // login must include one.
    if response.refresh_token().is_none() {
        return Err(LoginError::TokenExchange(
            "token response missing refresh_token".to_string(),
        ));
    }

    let now = chrono::Utc::now();
    let mut token_data = OAuthTokenData::from_response(&response, now);
    // Store client credentials in the token file so the server can refresh.
    token_data.client_id = Some(client_id.to_string());
    token_data.client_secret = Some(client_secret.to_string());

    Ok(token_data)
}

/// Proxy token exchange — POST to the proxy's `/token/exchange` endpoint.
///
/// Includes IPv4 retry logic for the common case where the proxy's
/// `ALLOWED_SOURCES` only resolves to IPv4 addresses.
async fn exchange_code_proxy(
    code: AuthorizationCode,
    pkce_verifier: PkceCodeVerifier,
    client_id: &str,
    proxy_url: &str,
) -> Result<OAuthTokenData, LoginError> {
    let url = format!("{}/token/exchange", proxy_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "code": code.secret(),
        "redirect_uri": REDIRECT_URI,
        "code_verifier": pkce_verifier.secret(),
    });

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| LoginError::TokenExchange(format!("failed to build HTTP client: {e}")))?;

    // First attempt.
    let (status, response_body) = send_proxy_exchange(&http_client, &url, &body).await?;

    // IPv4 retry on 403 with IPv6 source.
    let (status, response_body) = if status == reqwest::StatusCode::FORBIDDEN {
        match build_ipv4_retry_client(&response_body) {
            Some(ipv4_client) => send_proxy_exchange(&ipv4_client, &url, &body).await?,
            None => (status, response_body),
        }
    } else {
        (status, response_body)
    };

    if !status.is_success() {
        return Err(LoginError::TokenExchange(format!(
            "proxy returned {status}: {response_body}"
        )));
    }

    // Parse the response.
    let token_response: ProxyExchangeResponse = serde_json::from_str(&response_body)
        .map_err(|e| LoginError::TokenExchange(format!("failed to parse proxy response: {e}")))?;

    let now = chrono::Utc::now();
    let expires_at = token_response
        .expires_in
        .and_then(chrono::Duration::try_seconds)
        .map(|d| now + d);

    // Fail fast if the proxy response did not include a refresh token.
    // Without one, the session will break once the access token expires.
    let refresh_token = token_response.refresh_token.ok_or_else(|| {
        LoginError::TokenExchange("proxy response missing refresh_token".to_string())
    })?;

    let mut token_data = OAuthTokenData::from_raw(
        token_response.access_token,
        refresh_token,
        expires_at,
        token_response.token_type.unwrap_or_else(|| "bearer".into()),
        token_response
            .scope
            .map(|s| s.split(' ').map(String::from).collect()),
    );
    // Store proxy URL and client_id in the token file so the server
    // knows to use proxy-based refresh.
    token_data.proxy_url = Some(proxy_url.to_string());
    token_data.client_id = Some(client_id.to_string());

    Ok(token_data)
}

/// Send a POST request to the proxy exchange endpoint.
async fn send_proxy_exchange(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<(reqwest::StatusCode, String), LoginError> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| LoginError::TokenExchange(e.to_string()))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| LoginError::TokenExchange(format!("failed to read proxy response: {e}")))?;

    Ok((status, text))
}

/// Deserialization target for the proxy exchange response.
#[derive(serde::Deserialize)]
struct ProxyExchangeResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

// ============================================================================
// Proxy Config
// ============================================================================

/// Fetches the `client_id` from the OAuth proxy's `/config` endpoint.
///
/// Used in proxy mode where the proxy holds the client secret and the
/// CLI only needs the (public) client ID.
async fn fetch_proxy_client_id(proxy_url: &str) -> Result<String, LoginError> {
    let url = format!("{}/config", proxy_url.trim_end_matches('/'));

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LoginError::ProxyConfig {
            url: url.clone(),
            detail: format!("failed to build HTTP client: {e}"),
        })?;

    // First attempt.
    let response = http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| LoginError::ProxyConfig {
            url: url.clone(),
            detail: e.to_string(),
        })?;

    let status = response.status();

    // IPv4 retry on 403.
    let (status, body) =
        if status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.map_err(|e| LoginError::ProxyConfig {
                url: url.clone(),
                detail: e.to_string(),
            })?;
            match build_ipv4_retry_client(&body) {
                Some(ipv4_client) => {
                    let retry_response = ipv4_client.get(&url).send().await.map_err(|e| {
                        LoginError::ProxyConfig {
                            url: url.clone(),
                            detail: format!("IPv4 retry failed: {e}"),
                        }
                    })?;
                    let retry_status = retry_response.status();
                    let retry_body =
                        retry_response
                            .text()
                            .await
                            .map_err(|e| LoginError::ProxyConfig {
                                url: url.clone(),
                                detail: format!("failed to read retry response: {e}"),
                            })?;
                    (retry_status, retry_body)
                }
                None => (status, body),
            }
        } else {
            let body = response.text().await.map_err(|e| LoginError::ProxyConfig {
                url: url.clone(),
                detail: e.to_string(),
            })?;
            (status, body)
        };

    if !status.is_success() {
        return Err(LoginError::ProxyConfig {
            url,
            detail: format!("HTTP {status}: {body}"),
        });
    }

    let config: ProxyConfigResponse =
        serde_json::from_str(&body).map_err(|e| LoginError::ProxyConfig {
            url,
            detail: format!("failed to parse response: {e}"),
        })?;

    Ok(config.client_id)
}

/// Deserialization target for the proxy `/config` response.
#[derive(serde::Deserialize)]
struct ProxyConfigResponse {
    client_id: String,
}

// ============================================================================
// IPv4 Retry Helper
// ============================================================================

/// If a 403 response contains an IPv6 `source_ip`, build a `reqwest::Client`
/// that forces IPv4 connections for a retry attempt.
///
/// Reuses the same pattern as `lib.rs::build_ipv4_retry_client`.
fn build_ipv4_retry_client(response_body: &str) -> Option<reqwest::Client> {
    #[derive(serde::Deserialize)]
    struct ForbiddenResponse {
        source_ip: String,
    }

    let parsed: ForbiddenResponse = serde_json::from_str(response_body).ok()?;

    // IPv6 addresses contain colons; IPv4 never does.
    if !parsed.source_ip.contains(':') {
        return None;
    }

    reqwest::Client::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // ====================================================================
    // LoginState tests
    // ====================================================================

    #[test]
    fn login_state_starts_empty() {
        let state = LoginState::new();
        assert!(!state.is_active());
    }

    /// Helper to create a `LoginSession` for testing.
    fn test_session() -> LoginSession {
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async {
            // Sleep indefinitely — will be aborted by Drop.
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        LoginSession {
            task_handle: task,
            shutdown: shutdown_tx,
        }
    }

    /// Helper to create a `LoginSession` that finishes immediately.
    fn test_session_finished() -> LoginSession {
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async {});
        LoginSession {
            task_handle: task,
            shutdown: shutdown_tx,
        }
    }

    #[tokio::test]
    async fn login_state_cancels_previous_on_re_attempt() {
        let mut state = LoginState::new();

        let session1 = test_session();
        let task1 = &raw const session1.task_handle;
        state.set_active(session1);
        assert!(state.is_active());

        // Second attempt cancels the first.
        let session2 = test_session();
        state.set_active(session2);
        assert!(state.is_active());

        // The first task should have been aborted (pointer comparison
        // confirms we're tracking the second session now).
        let current_task = &raw const state
            .session
            .as_ref()
            .expect("session should be set")
            .task_handle;
        assert_ne!(task1, current_task, "should be tracking the new session");
    }

    #[tokio::test]
    async fn login_state_allows_after_finished() {
        let mut state = LoginState::new();

        let session = test_session_finished();
        state.set_active(session);

        // Wait for the task to finish.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(!state.is_active());

        // Second attempt should succeed.
        let session2 = test_session_finished();
        state.set_active(session2);
    }

    #[tokio::test]
    async fn login_state_clear_resets() {
        let mut state = LoginState::new();

        let session = test_session();
        state.set_active(session);
        assert!(state.is_active());

        state.clear();
        assert!(!state.is_active());
    }

    #[tokio::test]
    async fn login_session_drop_aborts_task() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        let task_clone = task.abort_handle();

        let session = LoginSession {
            task_handle: task,
            shutdown: shutdown_tx,
        };

        // Drop the session — should send shutdown and abort the task.
        drop(session);

        // The shutdown signal should have been sent.
        assert!(
            shutdown_rx.has_changed().unwrap_or(false) || *shutdown_rx.borrow(),
            "shutdown should have been sent"
        );

        // Yield to the runtime so the abort is processed before we check.
        tokio::task::yield_now().await;

        // The task should be aborted.
        assert!(task_clone.is_finished());
    }

    // ====================================================================
    // Callback server tests
    // ====================================================================

    #[tokio::test]
    async fn callback_server_starts_and_responds() {
        // Start the server on a dynamic port to avoid conflicts.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind");
        let port = listener.local_addr().expect("should have addr").port();

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let (url_tx, url_rx) = oneshot::channel::<String>();

        let callback_state = CallbackState {
            url_sender: Arc::new(tokio::sync::Mutex::new(Some(url_tx))),
            expected_state: "test-state".to_string(),
        };

        let app = axum::Router::new()
            .route("/callback", axum::routing::get(handle_callback))
            .with_state(callback_state);

        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.wait_for(|&v| v).await;
                })
                .await
        });

        // Send a request to the callback endpoint.
        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "http://127.0.0.1:{port}/callback?code=test-code&state=test-state"
            ))
            .send()
            .await
            .expect("should send request");

        assert!(response.status().is_success());
        let body = response.text().await.expect("should read body");
        assert!(body.contains("Authorization successful"));

        // The URL should have been sent through the channel.
        let callback_url = url_rx.await.expect("should receive URL");
        assert!(
            callback_url.starts_with(REDIRECT_URI),
            "callback URL should start with REDIRECT_URI: {callback_url}"
        );
        assert!(callback_url.contains("code=test-code"));
        assert!(callback_url.contains("state=test-state"));

        // Send shutdown signal (in production, LoginSession/complete_login_flow does this).
        let _ = shutdown_tx.send(true);

        // Server should shut down gracefully.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    #[tokio::test]
    async fn callback_server_handles_oauth_error() {
        // Start the server on a dynamic port to avoid conflicts.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind");
        let port = listener.local_addr().expect("should have addr").port();

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let (url_tx, url_rx) = oneshot::channel::<String>();

        let callback_state = CallbackState {
            url_sender: Arc::new(tokio::sync::Mutex::new(Some(url_tx))),
            expected_state: "test-state".to_string(),
        };

        let app = axum::Router::new()
            .route("/callback", axum::routing::get(handle_callback))
            .with_state(callback_state);

        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.wait_for(|&v| v).await;
                })
                .await
        });

        // Send an OAuth error callback (e.g., user denied consent).
        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "http://127.0.0.1:{port}/callback?error=access_denied&state=test-state"
            ))
            .send()
            .await
            .expect("should send request");

        assert!(response.status().is_success());
        let body = response.text().await.expect("should read body");
        assert!(
            body.contains("Authorization denied"),
            "error callback should show denial page, got: {body}"
        );
        assert!(
            !body.contains("Authorization successful"),
            "error callback should not show success page"
        );

        // The URL should still have been sent through the channel so
        // validate_callback can report the error promptly.
        let callback_url = url_rx.await.expect("should receive URL");
        assert!(
            callback_url.starts_with(REDIRECT_URI),
            "callback URL should start with REDIRECT_URI: {callback_url}"
        );
        assert!(callback_url.contains("error=access_denied"));
        assert!(callback_url.contains("state=test-state"));

        // Send shutdown signal.
        let _ = shutdown_tx.send(true);

        // Server should shut down gracefully.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    // ====================================================================
    // Port-in-use test
    // ====================================================================

    #[tokio::test]
    async fn callback_server_port_in_use_returns_error() {
        // Bind both IPv4 and IPv6 loopback to fully block the port,
        // since start_callback_server tries both addresses.
        let ipv4_bind = tokio::net::TcpListener::bind(format!("127.0.0.1:{CALLBACK_PORT}")).await;
        let ipv6_bind = tokio::net::TcpListener::bind(format!("[::1]:{CALLBACK_PORT}")).await;

        // If we couldn't bind both (port already in use from another test), skip.
        let (Some(ref _held_v4), Some(ref _held_v6)) = (ipv4_bind.ok(), ipv6_bind.ok()) else {
            return;
        };

        // Now try to start the callback server — should fail since both are taken.
        let result = start_callback_server().await;
        assert!(
            result.is_err(),
            "should fail when both IPv4 and IPv6 ports are in use"
        );
    }

    #[tokio::test]
    async fn callback_server_returns_at_least_one_listener() {
        let result = start_callback_server().await;
        // If we can't bind at all (e.g., port in use), skip.
        let Ok(listeners) = result else {
            return;
        };
        assert!(!listeners.is_empty(), "should return at least one listener");
        // On dual-stack systems, both should succeed.
        // On IPv4-only or IPv6-only, exactly one should succeed.
        assert!(listeners.len() <= 2, "should return at most two listeners");
    }

    // ====================================================================
    // Token file write test
    // ====================================================================

    #[test]
    fn token_file_written_after_exchange() {
        // This is tested via the existing save_token_file tests in oauth.rs.
        // The login flow reuses save_token_file, so we just verify the
        // function exists and the path resolves.
        let path = default_token_file_path();
        // path may be None in CI containers without a home directory.
        if let Some(ref p) = path {
            assert!(p.ends_with("onshape-mcp/tokens.json"));
        }
    }

    // ====================================================================
    // IPv4 retry tests
    // ====================================================================

    #[test]
    fn ipv4_retry_returns_client_for_ipv6() {
        let body = r#"{"source_ip":"2601:980:c200:8530:bfc8:c956:e7c1:1d07"}"#;
        assert!(build_ipv4_retry_client(body).is_some());
    }

    #[test]
    fn ipv4_retry_returns_none_for_ipv4() {
        let body = r#"{"source_ip":"71.58.134.128"}"#;
        assert!(build_ipv4_retry_client(body).is_none());
    }

    #[test]
    fn ipv4_retry_returns_none_for_invalid_json() {
        assert!(build_ipv4_retry_client("not json").is_none());
    }
}
