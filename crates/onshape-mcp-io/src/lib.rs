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
        CallToolRequestParams, CallToolResult, ErrorCode, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, ServerInfo,
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
use onshape_mcp_core::ValidationState;
use onshape_mcp_core::config::{AppConfig, AuthInventory, ResolvedAuth, TokenStatus, resolve_auth};
use onshape_mcp_core::openapi::OpenApiSpec;
use onshape_mcp_core::tools::{self, SideEffect, ToolResult};

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
    /// OAuth client ID (stored separately for state transition support).
    pub(crate) client_id: String,
    /// OAuth client secret (stored separately for state transition support).
    pub(crate) client_secret: SecretString,
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
    validation: Arc<tokio::sync::Mutex<ValidationState>>,
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
            validation: Arc::new(tokio::sync::Mutex::new(ValidationState::default())),
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

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult {
            resources: onshape_mcp_resources::list_resources(),
            next_cursor: None,
            meta: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let result = onshape_mcp_resources::read_resource(&request.uri);
        std::future::ready(match result {
            onshape_mcp_resources::ResourceResult::Immediate(Ok(read_result)) => Ok(read_result),
            onshape_mcp_resources::ResourceResult::Immediate(Err(
                onshape_mcp_resources::ResourceError::NotFound(uri),
            )) => Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("Resource not found: {uri}"),
                None::<serde_json::Value>,
            )),
        })
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Lock state to derive resolved auth and potentially execute API requests.
        let mut state = self.api_state.lock().await;
        let validation = self.validation.lock().await;
        let resolved_auth = state.resolved_auth();

        // Dispatch through core with the resolved auth and validation state.
        let result = tools::call_tool(
            &request.name,
            request.arguments.as_ref(),
            &resolved_auth,
            &validation,
            Some(&self.spec),
        );

        // Drop the validation lock before executing I/O — we'll re-lock
        // if we need to update it via side effects.
        drop(validation);

        // Dispatch loop: handles Immediate, OnshapeApiRequest, and
        // OnshapeApiRequestThen (which can chain multiple requests).
        let mut current = result;
        loop {
            // For variants that need API execution, check if credentials
            // are available. NotConfigured and OAuthPending cannot execute
            // API requests, so return informative tool-level errors.
            if matches!(
                current,
                ToolResult::OnshapeApiRequest { .. } | ToolResult::OnshapeApiRequestThen { .. }
            ) {
                match &*state {
                    ApiState::NotConfigured { .. } => return Ok(not_configured_error()),
                    ApiState::OAuthPending(_) => return Ok(oauth_pending_error()),
                    ApiState::Basic(_) | ApiState::OAuth(_) => {}
                }
            }

            match current {
                ToolResult::Immediate(r) => return r,
                ToolResult::OnshapeApiRequest { request: api_req } => {
                    // Simple case: execute and update implicit validation.
                    let raw = execute_raw_api_request(&mut state, &api_req).await;
                    match raw {
                        Ok(raw) => {
                            update_implicit_validation(&self.validation, raw.status).await;
                            return tools::process_api_response(raw.status, &raw.body);
                        }
                        Err(e) => return Err(e),
                    }
                }
                ToolResult::OnshapeApiRequestThen {
                    request: api_req,
                    then,
                } => {
                    // Execute the request, get raw response.
                    let raw = execute_raw_api_request(&mut state, &api_req).await;
                    match raw {
                        Ok(raw) => {
                            // Update implicit validation.
                            update_implicit_validation(&self.validation, raw.status).await;

                            // Invoke the callback.
                            let (next_result, side_effects) = then(raw.status, &raw.body);

                            // Apply side effects.
                            for effect in side_effects {
                                apply_side_effect(&self.validation, effect).await;
                            }

                            // Loop with the next result.
                            current = next_result;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
}

// ============================================================================
// API Request Execution
// ============================================================================

/// Result of executing a raw API request: HTTP status code and response body.
struct RawResponse {
    status: u16,
    body: String,
}

/// Error response when credentials are not configured.
fn not_configured_error() -> CallToolResult {
    CallToolResult {
        content: vec![rmcp::model::Content::text(
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
        content: vec![rmcp::model::Content::text(
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

/// Execute a raw API request, returning the HTTP status and body.
///
/// Handles authentication (Basic or OAuth with proactive/reactive refresh)
/// but does not process the response into a `CallToolResult`.
/// For OAuth, also handles permanent refresh failures by transitioning to
/// `OAuthPending` state.
///
/// Returns `None` for states where no API call can be made (`NotConfigured`,
/// `OAuthPending`). The caller should handle these with [`not_configured_error`]
/// or [`oauth_pending_error`] before calling this function.
async fn execute_raw_api_request(
    state: &mut ApiState,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<RawResponse, McpError> {
    match state {
        ApiState::NotConfigured { .. } | ApiState::OAuthPending(_) => {
            unreachable!("caller must check state before calling execute_raw_api_request")
        }
        ApiState::Basic(client) => execute_basic_raw(client, api_req).await,
        // OAuth needs &mut ApiState for the potential state transition,
        // so we handle it at the ApiState level.
        ApiState::OAuth(_) => execute_oauth_raw(state, api_req).await,
    }
}

/// Execute a raw request with Basic auth — returns status and body.
async fn execute_basic_raw(
    client: &OnshapeClient,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<RawResponse, McpError> {
    match client.execute(api_req).await {
        Ok(response) => Ok(RawResponse {
            status: response.status,
            body: response.body,
        }),
        Err(e) => Err(McpError::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("HTTP request failed: {e}"),
            None,
        )),
    }
}

/// Result of executing an OAuth request.
///
/// Separate from `Result` because we need to signal permanent refresh failures
/// that require the caller to transition the API state.
enum OAuthExecuteResult {
    /// Request completed (success or error).
    Ok(RawResponse),
    /// Request failed with an error.
    Err(McpError),
    /// Refresh token is permanently dead — caller should transition to `OAuthPending`.
    PermanentRefreshFailure {
        /// Human-readable error message for the user.
        message: String,
    },
}

/// Execute a raw request with OAuth, including proactive and reactive refresh.
///
/// Returns an `OAuthExecuteResult` which may signal permanent refresh failure.
async fn execute_oauth_inner(
    oauth: &mut OAuthApiState,
    api_req: &onshape_client_core::request::ApiRequest,
) -> OAuthExecuteResult {
    // Proactive refresh.
    let refreshed = oauth.session.pre_execute_action(chrono::Utc::now())
        == PreExecuteAction::RefreshNeeded
        && try_refresh(oauth).await.is_ok();

    let result = oauth.client.execute(api_req).await;

    // Reactive: retry on 401.
    if let Ok(ref response) = result
        && oauth
            .session
            .post_execute_action(response.status, refreshed)
            == PostExecuteAction::RefreshAndRetry
    {
        if let Err(e) = try_refresh(oauth).await {
            if matches!(e, RefreshError::PermanentExchange(_)) {
                return OAuthExecuteResult::PermanentRefreshFailure {
                    message: format!(
                        "OAuth refresh token is expired or revoked ({e}). \
                         You need to re-authenticate: complete the OAuth flow \
                         in your editor (e.g. run `opencode auth login`). \
                         The server will automatically detect new tokens once \
                         they are written."
                    ),
                };
            }
            return OAuthExecuteResult::Err(McpError::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("API returned 401 and token refresh failed: {e}"),
                None,
            ));
        }
        let retry = oauth.client.execute(api_req).await;
        return match retry {
            Ok(response) => OAuthExecuteResult::Ok(RawResponse {
                status: response.status,
                body: response.body,
            }),
            Err(e) => OAuthExecuteResult::Err(McpError::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("HTTP request failed on retry: {e}"),
                None,
            )),
        };
    }

    match result {
        Ok(response) => OAuthExecuteResult::Ok(RawResponse {
            status: response.status,
            body: response.body,
        }),
        Err(e) => OAuthExecuteResult::Err(McpError::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("HTTP request failed: {e}"),
            None,
        )),
    }
}

/// Execute a raw OAuth request, handling permanent refresh failures with
/// state transitions.
async fn execute_oauth_raw(
    state: &mut ApiState,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<RawResponse, McpError> {
    let ApiState::OAuth(oauth) = state else {
        return Err(McpError::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "expected OAuth state",
            None,
        ));
    };

    match execute_oauth_inner(oauth, api_req).await {
        OAuthExecuteResult::Ok(raw) => Ok(raw),
        OAuthExecuteResult::Err(e) => Err(e),
        OAuthExecuteResult::PermanentRefreshFailure { message } => {
            // Transition OAuth → OAuthPending.
            let ApiState::OAuth(oauth) = std::mem::replace(
                state,
                // Temporary placeholder — immediately replaced below.
                ApiState::NotConfigured {
                    configured_method: AuthMethod::OAuth,
                    detail: String::new(),
                },
            ) else {
                unreachable!("just checked this is OAuth");
            };
            *state = ApiState::OAuthPending(Box::new(OAuthPendingState {
                client_id: oauth.client_id,
                client_secret: oauth.client_secret,
                base_url: oauth.base_url,
                timeout: oauth.timeout,
                token_path: oauth.token_path,
            }));
            // Return the error as a raw 401 response so the caller can
            // process it appropriately.
            Ok(RawResponse {
                status: 401,
                body: message,
            })
        }
    }
}

// ============================================================================
// Validation State Helpers
// ============================================================================

/// Update implicit validation state based on an HTTP response status.
///
/// - 2xx → Valid (credentials confirmed working)
/// - 401 → Invalid (credentials confirmed bad)
/// - Other → No change (doesn't tell us about credential validity)
async fn update_implicit_validation(validation: &tokio::sync::Mutex<ValidationState>, status: u16) {
    if (200..300).contains(&status) {
        let mut v = validation.lock().await;
        *v = ValidationState {
            status: onshape_mcp_core::ValidationStatus::Valid,
            last_check: Some(chrono::Utc::now()),
            message: None,
        };
    } else if status == 401 {
        let mut v = validation.lock().await;
        *v = ValidationState {
            status: onshape_mcp_core::ValidationStatus::Invalid,
            last_check: Some(chrono::Utc::now()),
            message: Some("API returned 401 Unauthorized".into()),
        };
    }
    // Other statuses: don't change validation state.
}

/// Apply a side effect requested by a tool callback.
async fn apply_side_effect(validation: &tokio::sync::Mutex<ValidationState>, effect: SideEffect) {
    match effect {
        SideEffect::UpdateValidation(new_state) => {
            let mut v = validation.lock().await;
            *v = new_state;
        }
    }
}

// ============================================================================
// Token Refresh
// ============================================================================

/// Errors that can occur during a token refresh attempt.
#[derive(Debug, thiserror::Error)]
enum RefreshError {
    /// Transient exchange failure (network, server error).
    #[error("token refresh request failed: {0}")]
    Exchange(String),
    /// Permanent exchange failure (refresh token revoked/expired).
    #[error("token refresh permanently failed: {0}")]
    PermanentExchange(String),
    /// Failed to persist the refreshed tokens to disk.
    #[error("failed to save refreshed tokens: {0}")]
    Save(#[from] crate::oauth::TokenFileError),
    /// Failed to rebuild the HTTP client with the new token.
    #[error("failed to rebuild HTTP client: {0}")]
    RebuildClient(#[from] onshape_client_io::ClientError),
}

/// Check if a refresh error indicates a permanently dead refresh token.
///
/// OAuth error codes `unauthorized_client` and `invalid_grant` mean the
/// refresh token is revoked or expired — the user must re-authenticate.
fn is_permanent_refresh_failure(error_message: &str) -> bool {
    let lower = error_message.to_lowercase();
    lower.contains("unauthorized_client") || lower.contains("invalid_grant")
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
        .map_err(|e| {
            let msg = e.to_string();
            if is_permanent_refresh_failure(&msg) {
                RefreshError::PermanentExchange(msg)
            } else {
                RefreshError::Exchange(msg)
            }
        })?;

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
        client_id: client_id.clone(),
        client_secret: SecretString::from(client_secret.clone()),
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
    let watcher_handle = watcher_ctx.map(|ctx| {
        watcher::spawn_token_watcher(
            ctx,
            Arc::clone(&server.api_state),
            Arc::clone(&server.validation),
        )
    });

    // The watcher runs as a fire-and-forget background task. If it exits
    // (e.g. due to initialization failure), the server continues without
    // live token detection. Dropping the JoinHandle does not abort the task.
    let _watcher_handle = watcher_handle;

    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_permanent_refresh_failure tests ---

    #[test]
    fn permanent_refresh_unauthorized_client() {
        assert!(is_permanent_refresh_failure(
            "Server returned error response: unauthorized_client: Could not authenticate client"
        ));
    }

    #[test]
    fn permanent_refresh_invalid_grant() {
        assert!(is_permanent_refresh_failure(
            "Server returned error response: invalid_grant: Token has been revoked"
        ));
    }

    #[test]
    fn permanent_refresh_case_insensitive() {
        assert!(is_permanent_refresh_failure(
            "UNAUTHORIZED_CLIENT: something"
        ));
        assert!(is_permanent_refresh_failure("INVALID_GRANT: something"));
    }

    #[test]
    fn transient_refresh_network_error() {
        assert!(!is_permanent_refresh_failure(
            "error sending request: connection refused"
        ));
    }

    #[test]
    fn transient_refresh_server_error() {
        assert!(!is_permanent_refresh_failure(
            "Server returned error response: server_error: Internal error"
        ));
    }

    #[test]
    fn transient_refresh_generic_error() {
        assert!(!is_permanent_refresh_failure("something went wrong"));
    }
}
