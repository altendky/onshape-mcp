//! I/O layer for the Onshape MCP server.
//!
//! This crate provides the async runtime integration and MCP transport handling.
//! It delegates all tool logic to `onshape-mcp-core` and HTTP execution to
//! `onshape-client-io`.

pub mod config;
pub mod oauth;
pub mod watcher;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oauth2::AccessToken;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
        ServerInfo,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
};
use secrecy::{ExposeSecret, SecretString};

use onshape_client_core::auth::{AuthMethod, Credentials};
use onshape_client_core::oauth::{
    OAuthSession, OnshapeOAuthClient, PostExecuteAction, PreExecuteAction, default_token_file_path,
    onshape_oauth_client,
};
use onshape_client_io::{ClientAuthConfig, ClientConfig, OnshapeClient};
use onshape_mcp_core::config::{AppConfig, AuthInventory, ResolvedAuth, TokenStatus, resolve_auth};
use onshape_mcp_core::openapi::OpenApiSpec;
use onshape_mcp_core::tools::{self, ToolResult};

/// The embedded Onshape `OpenAPI` specification JSON.
///
/// Included at compile time from `onshape-openapi.json` in the crate root.
/// The spec is ~1.8 MB and adds to the binary size, but simplifies
/// distribution (single binary, no external files needed).
const OPENAPI_SPEC_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/onshape-openapi.json"));

/// Default refresh margin: start proactive refresh 60 seconds before expiry.
pub(crate) const REFRESH_MARGIN_SECS: i64 = 60;

// ============================================================================
// API State
// ============================================================================

/// Tracks the authentication state of the server.
///
/// `NotConfigured` — credentials are missing; API calls return an error.
/// `Basic` — API-key auth; no refresh needed.
/// `OAuth` — Bearer token auth with refresh capability.
/// `OAuthPending` — client creds present but awaiting tokens from auth flow.
pub(crate) enum ApiState {
    /// No credentials configured. API calls will return an informative error.
    NotConfigured {
        /// The configured auth method (for status reporting).
        configured_method: AuthMethod,
        /// Human-readable explanation of why credentials are not configured.
        detail: String,
    },
    /// Basic (API key) authentication — static credentials, no refresh.
    Basic(OnshapeClient),
    /// OAuth 2.0 bearer authentication with token refresh support.
    OAuth(Box<OAuthApiState>),
    /// OAuth client credentials present but no tokens yet.
    ///
    /// Holds enough information to transition to `OAuth` when tokens appear
    /// (via the file watcher detecting a new token file).
    OAuthPending(Box<OAuthPendingState>),
}

/// State for when OAuth client credentials are present but no tokens yet.
///
/// Contains the configuration needed to build a full `OAuthApiState`
/// when tokens become available (e.g. after the user completes the
/// OAuth authorization flow via the `OpenCode` plugin).
pub(crate) struct OAuthPendingState {
    /// OAuth client ID.
    pub(crate) client_id: String,
    /// OAuth client secret.
    pub(crate) client_secret: SecretString,
    /// Base URL for the Onshape API.
    pub(crate) base_url: String,
    /// HTTP request timeout.
    pub(crate) timeout: Duration,
    /// Path to the token file on disk.
    pub(crate) token_path: PathBuf,
}

impl ApiState {
    /// Derive the [`ResolvedAuth`] from the current state.
    fn resolved_auth(&self) -> ResolvedAuth {
        match self {
            Self::NotConfigured {
                configured_method,
                detail,
            } => ResolvedAuth::NotConfigured {
                configured_method: *configured_method,
                detail: detail.clone(),
            },
            Self::Basic(_) => ResolvedAuth::Basic,
            Self::OAuth(oauth) => ResolvedAuth::OAuthReady {
                expires_at: oauth.session.tokens.expires_at,
            },
            Self::OAuthPending(_) => ResolvedAuth::OAuthPending,
        }
    }
}

/// Mutable OAuth state: decision logic (core) + I/O resources.
pub(crate) struct OAuthApiState {
    /// Core: pure decision logic for token lifecycle.
    pub(crate) session: OAuthSession,
    /// Core: pure OAuth client config (endpoints, client credentials).
    pub(crate) oauth_client: OnshapeOAuthClient,
    /// I/O: HTTP client for Onshape API calls (carries the bearer token).
    pub(crate) client: OnshapeClient,
    /// I/O: HTTP client for token endpoint requests (no auth headers).
    pub(crate) refresh_http: reqwest::Client,
    /// I/O: path to the token file on disk.
    pub(crate) token_path: PathBuf,
    /// I/O: base URL for rebuilding the API client after refresh.
    pub(crate) base_url: String,
    /// I/O: timeout for rebuilding the API client after refresh.
    pub(crate) timeout: Duration,
}

impl OAuthApiState {
    /// Rebuild the API client with the current access token.
    ///
    /// Called after a successful token refresh to pick up the new token.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be constructed.
    pub(crate) fn rebuild_client(&mut self) -> Result<(), onshape_client_io::ClientError> {
        self.client = OnshapeClient::new(ClientConfig {
            base_url: self.base_url.clone(),
            auth: ClientAuthConfig::Bearer {
                access_token: AccessToken::new(self.session.access_token().secret().clone()),
            },
            timeout: Some(self.timeout),
        })?;
        Ok(())
    }
}

// ============================================================================
// Server
// ============================================================================

/// The MCP server handler for Onshape integration.
///
/// Uses `Arc<AppConfig>` because `SecretString` (used for API keys)
/// intentionally does not implement `Clone` to prevent secret proliferation.
///
/// `api_state` is behind a `tokio::sync::Mutex` because OAuth token refresh
/// requires `&mut` access. MCP over stdio is sequential, so there is
/// effectively no contention.
pub struct OnshapeMcpServer {
    info: ServerInfo,
    #[allow(dead_code)]
    config: Arc<AppConfig>,
    spec: Arc<OpenApiSpec>,
    api_state: Arc<tokio::sync::Mutex<ApiState>>,
}

impl OnshapeMcpServer {
    /// Creates a new server instance.
    ///
    /// If credentials are fully configured, an HTTP client is created for
    /// executing API calls. Otherwise, API call requests will return an
    /// informative error.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedded `OpenAPI` spec fails to parse or
    /// the HTTP client fails to initialize.
    pub fn new(
        name: &str,
        version: &str,
        config: AppConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let spec = OpenApiSpec::from_json(OPENAPI_SPEC_JSON)?;

        let api_state = build_api_state(&config, spec.server_url())?;

        Ok(Self {
            info: onshape_mcp_core::server_info(name, version),
            config: Arc::new(config),
            spec: Arc::new(spec),
            api_state: Arc::new(tokio::sync::Mutex::new(api_state)),
        })
    }
}

impl ServerHandler for OnshapeMcpServer {
    fn get_info(&self) -> ServerInfo {
        self.info.clone()
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        // Core returns Vec<Tool> directly - no conversion needed
        std::future::ready(Ok(ListToolsResult {
            tools: tools::list_tools(),
            next_cursor: None,
            meta: None,
        }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Lock state to derive resolved auth and potentially execute API requests.
        let mut state = self.api_state.lock().await;
        let resolved_auth = state.resolved_auth();

        // Dispatch through core with the resolved auth state.
        let result = tools::call_tool(
            &request.name,
            request.arguments.as_ref(),
            &resolved_auth,
            Some(&self.spec),
        );

        match result {
            ToolResult::Immediate(r) => r,
            ToolResult::OnshapeApiRequest { request: api_req } => {
                execute_api_request(&mut state, &api_req).await
            }
        }
    }
}

// ============================================================================
// API Request Execution
// ============================================================================

/// Execute an API request, handling authentication state and token refresh.
async fn execute_api_request(
    state: &mut ApiState,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<CallToolResult, McpError> {
    match state {
        ApiState::NotConfigured { .. } => Ok(not_configured_error()),
        ApiState::Basic(client) => execute_basic(client, api_req).await,
        ApiState::OAuth(oauth) => execute_oauth(oauth, api_req).await,
        ApiState::OAuthPending(_) => Ok(oauth_pending_error()),
    }
}

/// Error response when credentials are not configured.
fn not_configured_error() -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(
            "Cannot execute API call: credentials are not configured. \
             Set access_key and secret_key via config file, environment \
             variables, or CLI flags. For OAuth, run the authorization flow \
             to obtain tokens.",
        )],
        is_error: Some(true),
        structured_content: None,
        meta: None,
    }
}

/// Error response when OAuth is pending (client creds present but no tokens).
fn oauth_pending_error() -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(
            "Cannot execute API call: OAuth authorization not yet completed. \
             Complete the OAuth flow in your editor (e.g. via the OpenCode plugin) \
             to obtain access tokens. The server will automatically detect the \
             new tokens once they are written.",
        )],
        is_error: Some(true),
        structured_content: None,
        meta: None,
    }
}

/// Execute a request with Basic (API key) auth — no refresh logic.
async fn execute_basic(
    client: &OnshapeClient,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<CallToolResult, McpError> {
    match client.execute(api_req).await {
        Ok(response) => tools::process_api_response(response.status, &response.body),
        Err(e) => Ok(CallToolResult {
            content: vec![Content::text(format!("HTTP request failed: {e}"))],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        }),
    }
}

/// Execute a request with OAuth, including proactive and reactive refresh.
async fn execute_oauth(
    oauth: &mut OAuthApiState,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<CallToolResult, McpError> {
    // Proactive: core says whether to refresh before the request.
    // If refresh fails, proceed with the current token — it might still work.
    let refreshed = oauth.session.pre_execute_action(chrono::Utc::now())
        == PreExecuteAction::RefreshNeeded
        && try_refresh(oauth).await.is_ok();

    let result = oauth.client.execute(api_req).await;

    // Reactive: core says whether to retry on 401.
    if let Ok(ref response) = result
        && oauth
            .session
            .post_execute_action(response.status, refreshed)
            == PostExecuteAction::RefreshAndRetry
    {
        if let Err(e) = try_refresh(oauth).await {
            return Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "API returned 401 and token refresh failed: {e}"
                ))],
                is_error: Some(true),
                structured_content: None,
                meta: None,
            });
        }
        // Retry with the refreshed token.
        let retry = oauth.client.execute(api_req).await;
        return format_execute_result(retry);
    }

    format_execute_result(result)
}

/// Format an `OnshapeClient::execute` result into a `CallToolResult`.
fn format_execute_result(
    result: Result<onshape_client_core::request::ApiResponse, onshape_client_io::ClientError>,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(response) => tools::process_api_response(response.status, &response.body),
        Err(e) => Ok(CallToolResult {
            content: vec![Content::text(format!("HTTP request failed: {e}"))],
            is_error: Some(true),
            structured_content: None,
            meta: None,
        }),
    }
}

// ============================================================================
// Token Refresh
// ============================================================================

/// Errors that can occur during a token refresh attempt.
#[derive(Debug, thiserror::Error)]
enum RefreshError {
    /// The oauth2 token exchange failed.
    #[error("token refresh request failed: {0}")]
    Exchange(String),
    /// Failed to persist the refreshed tokens to disk.
    #[error("failed to save refreshed tokens: {0}")]
    Save(#[from] crate::oauth::TokenFileError),
    /// Failed to rebuild the HTTP client with the new token.
    #[error("failed to rebuild HTTP client: {0}")]
    RebuildClient(#[from] onshape_client_io::ClientError),
}

/// Attempt to refresh the OAuth access token.
///
/// 1. Check if an external process already refreshed (token file reload).
/// 2. If not, perform the HTTP refresh via the oauth2 crate.
/// 3. Apply the response, persist to disk, rebuild the API client.
async fn try_refresh(oauth: &mut OAuthApiState) -> Result<(), RefreshError> {
    // 1. Check if external process already refreshed (token file reload).
    if let Ok(file_tokens) = crate::oauth::load_token_file(&oauth.token_path)
        && oauth
            .session
            .apply_external_tokens(file_tokens, chrono::Utc::now())
    {
        oauth.rebuild_client()?;
        return Ok(());
    }

    // 2. Do the HTTP refresh via oauth2.
    let response = oauth
        .oauth_client
        .exchange_refresh_token(oauth.session.refresh_token())
        .request_async(&oauth.refresh_http)
        .await
        .map_err(|e| RefreshError::Exchange(e.to_string()))?;

    // 3. Apply response (pure core logic).
    oauth.session.apply_refresh(&response, chrono::Utc::now());

    // 4. Persist to disk.
    crate::oauth::save_token_file(&oauth.token_path, &oauth.session.tokens)?;

    // 5. Rebuild API client with new access token.
    oauth.rebuild_client()?;

    Ok(())
}

// ============================================================================
// State Construction
// ============================================================================

/// Probe the token file status for OAuth authentication.
///
/// Returns `TokenStatus::Present` if the file exists and can be parsed,
/// or `TokenStatus::Absent` otherwise.
fn probe_token_status(token_path: Option<&std::path::Path>) -> TokenStatus {
    let Some(path) = token_path else {
        return TokenStatus::Absent;
    };
    match crate::oauth::load_token_file(path) {
        Ok(data) => TokenStatus::Present {
            expires_at: data.expires_at,
        },
        Err(_) => TokenStatus::Absent,
    }
}

/// Build the initial API state from configuration.
///
/// Uses the core's auth resolution logic to determine which state to build:
/// - `Basic` — both API keys present
/// - `OAuthReady` — client credentials + token file present
/// - `OAuthPending` — client credentials but no tokens yet
/// - `NotConfigured` — no usable credentials
fn build_api_state(
    config: &AppConfig,
    server_url: &str,
) -> Result<ApiState, Box<dyn std::error::Error + Send + Sync>> {
    let token_path = default_token_file_path();
    let token_status = probe_token_status(token_path.as_deref());
    let inventory = AuthInventory::from_config(&config.auth, token_status);
    let resolved = resolve_auth(config.auth.method, &inventory);

    match resolved {
        ResolvedAuth::Basic => {
            // Both keys are guaranteed present by the resolution logic.
            let (Some(access_key), Some(secret_key)) =
                (&config.auth.access_key, &config.auth.secret_key)
            else {
                unreachable!("ResolvedAuth::Basic but keys are None");
            };

            let client = OnshapeClient::new(ClientConfig {
                base_url: server_url.to_string(),
                auth: ClientAuthConfig::Basic {
                    credentials: Arc::new(Credentials {
                        access_key: SecretString::from(access_key.clone()),
                        secret_key: SecretString::from(secret_key.clone()),
                    }),
                },
                timeout: Some(config.http.timeout),
            })?;
            Ok(ApiState::Basic(client))
        }
        ResolvedAuth::OAuthReady { .. } => {
            build_oauth_ready_state(config, server_url, token_path.as_deref())
        }
        ResolvedAuth::OAuthPending => Ok(build_oauth_pending_state(config, server_url, token_path)),
        ResolvedAuth::NotConfigured {
            configured_method,
            detail,
        } => Ok(ApiState::NotConfigured {
            configured_method,
            detail,
        }),
    }
}

/// Build the `OAuthReady` API state from config and token file.
///
/// The token file is guaranteed to exist by the resolution logic.
/// If the token is expired, proactive refresh will fire on the first request.
fn build_oauth_ready_state(
    config: &AppConfig,
    server_url: &str,
    token_path: Option<&std::path::Path>,
) -> Result<ApiState, Box<dyn std::error::Error + Send + Sync>> {
    let (Some(client_id), Some(client_secret)) =
        (&config.auth.client_id, &config.auth.client_secret)
    else {
        unreachable!("OAuthReady but OAuth fields are None");
    };

    let Some(token_path) = token_path else {
        unreachable!("OAuthReady but token path is None");
    };

    let token_data = crate::oauth::load_token_file(token_path)
        .map_err(|e| format!("failed to load token file: {e}"))?;

    let oauth_client = onshape_oauth_client(client_id, client_secret.expose_secret());
    let session = OAuthSession::new(token_data, chrono::Duration::seconds(REFRESH_MARGIN_SECS));

    let timeout = config.http.timeout;
    let base_url = server_url.to_string();
    let client = OnshapeClient::new(ClientConfig {
        base_url: base_url.clone(),
        auth: ClientAuthConfig::Bearer {
            access_token: AccessToken::new(session.access_token().secret().clone()),
        },
        timeout: Some(timeout),
    })?;

    let refresh_http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build refresh HTTP client: {e}"))?;

    Ok(ApiState::OAuth(Box::new(OAuthApiState {
        session,
        oauth_client,
        client,
        refresh_http,
        token_path: token_path.to_path_buf(),
        base_url,
        timeout,
    })))
}

/// Build the `OAuthPending` state when client credentials are present
/// but no token file exists yet.
fn build_oauth_pending_state(
    config: &AppConfig,
    server_url: &str,
    token_path: Option<PathBuf>,
) -> ApiState {
    let (Some(client_id), Some(client_secret)) =
        (&config.auth.client_id, &config.auth.client_secret)
    else {
        unreachable!("OAuthPending but OAuth fields are None");
    };

    let Some(token_path) = token_path else {
        // Can't watch for tokens without a path — fall back to NotConfigured
        return ApiState::NotConfigured {
            configured_method: config.auth.method,
            detail: "OAuth pending but no token file path available".into(),
        };
    };

    ApiState::OAuthPending(Box::new(OAuthPendingState {
        client_id: client_id.clone(),
        client_secret: SecretString::from(client_secret.clone()),
        base_url: server_url.to_string(),
        timeout: config.http.timeout,
        token_path,
    }))
}

// ============================================================================
// Entry Point
// ============================================================================

/// Runs the MCP server on stdio transport.
///
/// # Arguments
///
/// * `name` - The server name (typically from `CARGO_PKG_NAME`)
/// * `version` - The server version (typically from `CARGO_PKG_VERSION`)
/// * `config` - Application configuration (loaded by the binary crate)
///
/// # Errors
///
/// Returns an error if the server fails to start or encounters a fatal error.
pub async fn run(
    name: &str,
    version: &str,
    config: AppConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = OnshapeMcpServer::new(name, version, config)?;

    // Watch the token file for changes. This is useful for:
    // - NotConfigured: detect token file with embedded credentials appearing
    // - OAuthPending: detect tokens appearing after the user completes auth
    // - OAuth: detect external token refreshes
    //
    // For NotConfigured state, the token file (and its directory) may not
    // exist yet, so we derive the path from platform defaults and ensure the
    // directory exists.
    let token_path = {
        let state = server.api_state.lock().await;
        match &*state {
            ApiState::OAuthPending(pending) => Some(pending.token_path.clone()),
            ApiState::OAuth(oauth) => Some(oauth.token_path.clone()),
            ApiState::NotConfigured { .. } => default_token_file_path(),
            ApiState::Basic(_) => None,
        }
    };
    let watcher_ctx = token_path.map(|token_path| {
        // Ensure the data directory exists so the watcher can monitor it.
        if let Some(parent) = token_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        watcher::WatcherContext {
            token_path,
            base_url: server.spec.server_url().to_string(),
            timeout: server.config.http.timeout,
        }
    });
    let watcher_handle =
        watcher_ctx.map(|ctx| watcher::spawn_token_watcher(ctx, Arc::clone(&server.api_state)));

    // The watcher runs as a fire-and-forget background task. If it exits
    // (e.g. due to initialization failure), the server continues without
    // live token detection. Dropping the JoinHandle does not abort the task.
    let _watcher_handle = watcher_handle;

    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}
