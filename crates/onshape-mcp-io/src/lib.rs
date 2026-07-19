//! I/O layer for the Onshape MCP server.
//!
//! This crate provides the async runtime integration and MCP transport handling.
//! It delegates all tool logic to `onshape-mcp-core` and HTTP execution to
//! `onshape-client-io`.

pub mod config;
pub mod login;
pub mod oauth;
pub mod oauth_server;
pub mod watcher;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use oauth2::AccessToken;
use oauth2_reqwest::ReqwestClient;
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
    OAuthSession, OnshapeOAuthClient, PostExecuteAction, PreExecuteAction, onshape_oauth_client,
};
use onshape_client_core::request::ApiResponse;
use onshape_client_io::{ClientAuthConfig, ClientConfig, OnshapeClient};
use onshape_mcp_core::ValidationState;
use onshape_mcp_core::config::{AppConfig, AuthInventory, ResolvedAuth, TokenStatus, resolve_auth};
use onshape_mcp_core::tools::{self, IoResult, SideEffect, ToolEffect};
use onshape_openapi::OpenApiSpec;

use crate::oauth::{McpOAuthTokenFile, McpOAuthTokenMetadata, default_token_file_path};

/// The embedded Onshape `OpenAPI` specification JSON.
///
/// Included at compile time from `onshape-openapi.json` in the crate root.
/// The spec is ~1.8 MB and adds to the binary size, but simplifies
/// distribution (single binary, no external files needed).
const OPENAPI_SPEC_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/onshape-openapi.json"));

/// MCP compatibility fallback for embedded specs that predate an explicit server URL.
const OPENAPI_SERVER_URL_FALLBACK: &str = "https://cad.onshape.com/api/v6";

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
    /// Credentials are not configured or are incomplete.
    NotConfigured {
        /// What auth method was attempted.
        configured_method: AuthMethod,
        /// Human-readable detail about what's missing.
        detail: String,
    },
    /// Basic (API key) auth — a static client that never refreshes.
    Basic(OnshapeClient),
    /// OAuth with tokens — can proactively and reactively refresh.
    OAuth(Box<OAuthApiState>),
    /// OAuth client credentials present, but no tokens yet.
    OAuthPending(Box<OAuthPendingState>),
    /// Per-user OAuth for the HTTP transport — refreshes via the shared
    /// [`OAuthServerState`](oauth_server::OAuthServerState).
    HttpOAuth(Box<HttpOAuthApiState>),
}

/// How the server refreshes OAuth tokens.
///
/// - `Direct`: the server contacts Onshape's token endpoint using the
///   `client_id` and `client_secret` (via the `oauth2` crate).
/// - `Proxy`: the server sends the refresh token to a configured self-hosted OAuth
///   proxy which adds the client credentials and forwards to Onshape.
pub(crate) enum RefreshMethod {
    /// Direct refresh — client secret held locally.
    Direct {
        /// Pre-configured `OAuth2` client (endpoints + credentials).
        oauth_client: Box<OnshapeOAuthClient>,
        /// OAuth client ID (kept for state-transition support).
        client_id: String,
        /// OAuth client secret (kept for state-transition support).
        client_secret: SecretString,
    },
    /// Proxy refresh — client secret held by the proxy.
    Proxy {
        /// Base URL of the explicitly configured self-hosted OAuth proxy.
        proxy_url: String,
    },
}

impl RefreshMethod {
    /// Extract token-file metadata that should be preserved for this refresh mode.
    fn token_metadata_from_file(&self, token_file: &McpOAuthTokenFile) -> McpOAuthTokenMetadata {
        match self {
            Self::Direct {
                client_id,
                client_secret,
                ..
            } => McpOAuthTokenMetadata {
                client_id: Some(client_id.clone()),
                client_secret: Some(client_secret.expose_secret().to_string()),
                proxy_url: None,
            },
            Self::Proxy { proxy_url } => McpOAuthTokenMetadata {
                client_id: token_file.client_id.clone(),
                client_secret: None,
                proxy_url: Some(proxy_url.clone()),
            },
        }
    }
}

pub(crate) fn refresh_method_from_token_file(
    token_file: &McpOAuthTokenFile,
) -> Result<Option<RefreshMethod>, String> {
    let proxy_url = token_file
        .proxy_url
        .as_ref()
        .filter(|url| !url.trim().is_empty());
    let client_secret = token_file
        .client_secret
        .as_ref()
        .filter(|secret| !secret.trim().is_empty());
    if proxy_url.is_some() && client_secret.is_some() {
        return Err(
            "token file must not contain both nonblank client_secret and proxy_url".to_string(),
        );
    }

    if let Some(proxy_url) = proxy_url {
        return onshape_mcp_core::validate_proxy_url(proxy_url)
            .map(|proxy_url| Some(RefreshMethod::Proxy { proxy_url }));
    }

    let (Some(client_id), Some(client_secret)) = (
        token_file
            .client_id
            .as_ref()
            .filter(|id| !id.trim().is_empty()),
        client_secret,
    ) else {
        return Ok(None);
    };

    Ok(Some(RefreshMethod::Direct {
        oauth_client: Box::new(onshape_oauth_client(client_id, client_secret)),
        client_id: client_id.clone(),
        client_secret: SecretString::from(client_secret.clone()),
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExternalTokenAdoptionPolicy {
    WatcherObservedWrite,
    #[cfg(test)]
    RoutineRefreshRead,
}

pub(crate) fn adopt_external_token_file(
    oauth: &mut OAuthApiState,
    token_file: &McpOAuthTokenFile,
    policy: ExternalTokenAdoptionPolicy,
) -> Result<bool, onshape_client_io::ClientError> {
    if !crate::oauth::has_complete_token_material(token_file) {
        return Ok(false);
    }
    let refresh_method = refresh_method_from_token_file(token_file).map_err(|message| {
        onshape_client_io::ClientError::RequestBuild {
            message: format!("invalid OAuth refresh metadata in token file: {message}"),
        }
    })?;
    let authoritative = match policy {
        ExternalTokenAdoptionPolicy::WatcherObservedWrite => true,
        #[cfg(test)]
        ExternalTokenAdoptionPolicy::RoutineRefreshRead => false,
    };
    if authoritative && refresh_method.is_none() {
        return Ok(false);
    }
    let refresh_mode_changed = refresh_method.as_ref().is_some_and(|method| {
        std::mem::discriminant(method) != std::mem::discriminant(&oauth.refresh_method)
    });

    if authoritative {
        oauth.session.tokens = token_file.tokens.clone();
    } else if refresh_mode_changed
        || !oauth
            .session
            .apply_external_tokens(token_file.tokens.clone(), chrono::Utc::now())
    {
        return Ok(false);
    }

    if let Some(refresh_method) = refresh_method {
        oauth.refresh_method = refresh_method;
    }

    oauth.token_metadata = oauth.refresh_method.token_metadata_from_file(token_file);
    oauth.rebuild_client()?;
    Ok(true)
}

/// State for when OAuth client credentials are present but no tokens yet.
///
/// Contains the configuration needed to build a full `OAuthApiState`
/// when tokens become available (e.g. after the user completes the
/// OAuth authorization flow via the `OpenCode` plugin).
pub(crate) struct OAuthPendingState {
    /// How to refresh tokens once they arrive.
    pub(crate) refresh_method: PendingRefreshMethod,
    /// Base URL for the Onshape API.
    pub(crate) base_url: String,
    /// HTTP request timeout.
    pub(crate) timeout: Duration,
    /// Path to the token file on disk.
    pub(crate) token_path: PathBuf,
}

/// Pending-state variant of [`RefreshMethod`].
///
/// Same concept but without the pre-built `OnshapeOAuthClient`
/// (which requires a client to exist, pointless while pending).
pub(crate) enum PendingRefreshMethod {
    /// Direct — `client_id` + `client_secret` known, awaiting tokens.
    Direct {
        client_id: String,
        client_secret: SecretString,
    },
    /// Proxy — proxy URL known, awaiting tokens.
    Proxy { proxy_url: String },
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
            Self::HttpOAuth(http_oauth) => ResolvedAuth::OAuthReady {
                expires_at: http_oauth.expires_at,
            },
        }
    }
}

/// Mutable OAuth state: decision logic (core) + I/O resources.
pub(crate) struct OAuthApiState {
    /// Core: pure decision logic for token lifecycle.
    pub(crate) session: OAuthSession,
    /// MCP-owned persistence metadata for the token file.
    pub(crate) token_metadata: McpOAuthTokenMetadata,
    /// How to refresh tokens (direct or via proxy).
    pub(crate) refresh_method: RefreshMethod,
    /// I/O: HTTP client for Onshape API calls (carries the bearer token).
    pub(crate) client: OnshapeClient,
    /// I/O: HTTP client for token endpoint / proxy requests (no auth headers).
    pub(crate) refresh_http: reqwest::Client,
    /// I/O: path to the token file on disk.
    pub(crate) token_path: PathBuf,
    /// Last token-file publication incorporated into this state.
    pub(crate) last_observed_token_snapshot: TokenFileSnapshot,
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

/// Per-user OAuth state for the HTTP transport.
///
/// Unlike [`OAuthApiState`] (which is single-user, file-based, long-lived),
/// this is created per-request and delegates refresh to the shared
/// [`OAuthServerState`] which holds the server's Onshape client credentials
/// and per-user token storage.
pub(crate) struct HttpOAuthApiState {
    /// HTTP client for Onshape API calls (carries the bearer token).
    client: OnshapeClient,
    /// Onshape user ID (key into the shared token store).
    user_id: String,
    /// When the current access token expires, if known.
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Shared OAuth server state (holds client credentials and token store).
    oauth_state: Arc<oauth_server::OAuthServerState>,
    /// Base URL for rebuilding the API client after refresh.
    base_url: String,
    /// HTTP request timeout for rebuilding the API client after refresh.
    timeout: Duration,
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
    login_state: Arc<tokio::sync::Mutex<login::LoginState>>,
    /// Shared OAuth server state for per-user token refresh (HTTP transport only).
    oauth_state: Option<Arc<oauth_server::OAuthServerState>>,
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
        let spec = OpenApiSpec::from_json_with_server_url_fallback(
            OPENAPI_SPEC_JSON,
            OPENAPI_SERVER_URL_FALLBACK,
        )?;

        let api_state = build_api_state(&config, spec.server_url())?;

        Ok(Self {
            info: onshape_mcp_core::server_info(name, version),
            config: Arc::new(config),
            spec: Arc::new(spec),
            api_state: Arc::new(tokio::sync::Mutex::new(api_state)),
            validation: Arc::new(tokio::sync::Mutex::new(ValidationState::default())),
            login_state: Arc::new(tokio::sync::Mutex::new(login::LoginState::new())),
            oauth_state: None,
        })
    }

    /// Creates a new server instance from pre-built shared state.
    ///
    /// Used by `run_http()` so the factory closure can clone `Arc`s cheaply.
    pub(crate) fn from_shared_state(
        info: ServerInfo,
        config: Arc<AppConfig>,
        spec: Arc<OpenApiSpec>,
        api_state: Arc<tokio::sync::Mutex<ApiState>>,
        validation: Arc<tokio::sync::Mutex<ValidationState>>,
        oauth_state: Option<Arc<oauth_server::OAuthServerState>>,
    ) -> Self {
        Self {
            info,
            config,
            spec,
            api_state,
            validation,
            login_state: Arc::new(tokio::sync::Mutex::new(login::LoginState::new())),
            oauth_state,
        }
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
        std::future::ready(Ok(ListToolsResult::with_all_items(tools::list_tools())))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(
            onshape_mcp_resources::list_resources(),
        )))
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Check if we're in HTTP mode by looking for UserContext in the
        // request extensions (injected by the auth middleware via the
        // Streamable HTTP transport's `http::request::Parts`).
        let user_ctx = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<oauth_server::UserContext>())
            .cloned();

        if let Some(user_ctx) = user_ctx {
            return self.call_tool_http(request, &user_ctx).await;
        }

        // stdio mode: use shared credentials (existing path).
        self.call_tool_stdio(request).await
    }
}

// ============================================================================
// Transport-Specific call_tool Implementations
// ============================================================================

impl OnshapeMcpServer {
    /// Handle `call_tool` in stdio mode using shared credentials.
    #[allow(clippy::significant_drop_tightening)]
    async fn call_tool_stdio(
        &self,
        request: CallToolRequestParams,
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

        // Dispatch loop: handles Done, ApiRequest (which can chain
        // multiple requests via resume()), and other effects.
        dispatch_tool_effect(
            result,
            &mut state,
            &self.validation,
            Some(&self.login_state),
            true, // stdio: file writes allowed
            true, // stdio: file reads allowed
        )
        .await
    }

    /// Handle `call_tool` in HTTP mode using per-user credentials.
    async fn call_tool_http(
        &self,
        request: CallToolRequestParams,
        user_ctx: &oauth_server::UserContext,
    ) -> Result<CallToolResult, McpError> {
        let validation = self.validation.lock().await;

        // In HTTP mode, we present OAuthReady with the user's token expiry.
        let resolved_auth = ResolvedAuth::OAuthReady {
            expires_at: user_ctx.onshape_tokens.expires_at(),
        };

        let result = tools::call_tool(
            &request.name,
            request.arguments.as_ref(),
            &resolved_auth,
            &validation,
            Some(&self.spec),
        );

        drop(validation);

        // Build a per-user API client with refresh capability.
        let client = OnshapeClient::new(ClientConfig {
            base_url: self.spec.server_url().to_string(),
            auth: ClientAuthConfig::Bearer {
                access_token: AccessToken::new(
                    user_ctx
                        .onshape_tokens
                        .access_token()
                        .expose_secret()
                        .to_string(),
                ),
            },
            timeout: Some(self.config.api.timeout),
        })
        .map_err(|e| {
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("failed to build per-user API client: {e}"),
                None,
            )
        })?;

        let mut api_state = if let Some(oauth_state) = &self.oauth_state {
            ApiState::HttpOAuth(Box::new(HttpOAuthApiState {
                client,
                user_id: user_ctx.user_id.clone(),
                expires_at: user_ctx.onshape_tokens.expires_at(),
                oauth_state: Arc::clone(oauth_state),
                base_url: self.spec.server_url().to_string(),
                timeout: self.config.api.timeout,
            }))
        } else {
            ApiState::Basic(client)
        };

        dispatch_tool_effect(result, &mut api_state, &self.validation, None, false, false).await
    }
}

/// Shared dispatch loop for tool effects.
///
/// Handles `Done`, `ApiRequest`, `OAuthLoginFlow`, `WriteFiles`, and `ReadFiles`
/// variants. Used by both stdio and HTTP modes.
///
/// After executing an `ApiRequest`, `WriteFiles`, or `ReadFiles` effect, calls
/// [`tools::resume()`] with the continuation and the I/O result to get the
/// next effect, then loops.
///
/// `allow_file_writes` and `allow_file_reads` control whether file I/O effects
/// are executed. The stdio transport passes `true` for both (local, single-user
/// process); the HTTP transport passes `false` to prevent network-facing
/// file-system access.
#[allow(clippy::significant_drop_tightening)]
async fn dispatch_tool_effect(
    initial_effect: ToolEffect,
    state: &mut ApiState,
    validation: &tokio::sync::Mutex<ValidationState>,
    login_state: Option<&tokio::sync::Mutex<login::LoginState>>,
    allow_file_writes: bool,
    allow_file_reads: bool,
) -> Result<CallToolResult, McpError> {
    let mut current = initial_effect;
    loop {
        // For variants that need API execution, check if credentials
        // are available. NotConfigured and OAuthPending cannot execute
        // API requests, so return informative tool-level errors.
        if matches!(current, ToolEffect::ApiRequest { .. }) {
            match state {
                ApiState::NotConfigured { .. } => return Ok(not_configured_error()),
                ApiState::OAuthPending(_) => return Ok(oauth_pending_error()),
                ApiState::Basic(_) | ApiState::OAuth(_) | ApiState::HttpOAuth(_) => {}
            }
        }

        match current {
            ToolEffect::Done(r) => return r,
            ToolEffect::OAuthLoginFlow { mode } => {
                // In HTTP mode, login_state is None — return informative message.
                let Some(login_state) = login_state else {
                    return Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                        "Authentication is handled via the browser OAuth flow \
                         when connecting to this server. You do not need to \
                         run the login tool manually.",
                    )]));
                };
                return handle_oauth_login_flow(mode, login_state).await;
            }
            ToolEffect::ApiRequest {
                request: api_req,
                continuation,
            } => {
                let raw = execute_raw_api_request(state, &api_req).await;
                match raw {
                    Ok(raw) => {
                        update_implicit_validation(validation, raw.status).await;

                        let (next_effect, side_effects) =
                            resume_with_raw_response(continuation, &raw);

                        for effect in side_effects {
                            apply_side_effect(validation, effect).await;
                        }

                        current = next_effect;
                    }
                    Err(e) => return Err(e),
                }
            }
            ToolEffect::WriteFiles {
                files,
                continuation,
            } => {
                if !allow_file_writes {
                    return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                        "File write operations are not supported over the HTTP transport. \
                         The onshape_screenshot tool's output_path parameter requires the \
                         stdio transport (local process).",
                    )]));
                }
                let results = write_files(&files).await;

                let (next_effect, side_effects) =
                    tools::resume(continuation, IoResult::FileWriteResults(&results));

                for effect in side_effects {
                    apply_side_effect(validation, effect).await;
                }

                current = next_effect;
            }
            ToolEffect::ReadFiles {
                reads,
                continuation,
            } => {
                if !allow_file_reads {
                    return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                        "File read operations are not supported over the HTTP transport. \
                         File references in onshape_api_call require the stdio transport \
                         (local process).",
                    )]));
                }
                let results = read_files(&reads).await;

                let (next_effect, side_effects) =
                    tools::resume(continuation, IoResult::FileReadResults(&results));

                for effect in side_effects {
                    apply_side_effect(validation, effect).await;
                }

                current = next_effect;
            }
        }
    }
}

// ============================================================================
// File Write Execution
// ============================================================================

/// Write files to disk as requested by [`ToolEffect::WriteFiles`].
///
/// Creates the parent directory for each file if it does not exist.
/// Returns one [`tools::FileWriteResult`] per input file.
async fn write_files(files: &[tools::FileWrite]) -> Vec<tools::FileWriteResult> {
    let mut results = Vec::with_capacity(files.len());
    for file in files {
        // Ensure the parent directory exists.
        if let Some(parent) = file.path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            results.push(tools::FileWriteResult::Error {
                path: file.path.clone(),
                message: format!("failed to create directory {}: {e}", parent.display()),
            });
            continue;
        }

        match tokio::fs::write(&file.path, &file.data).await {
            Ok(()) => {
                results.push(tools::FileWriteResult::Success {
                    path: file.path.clone(),
                });
            }
            Err(e) => {
                results.push(tools::FileWriteResult::Error {
                    path: file.path.clone(),
                    message: format!("failed to write file: {e}"),
                });
            }
        }
    }
    results
}

// ============================================================================
// File Read Execution
// ============================================================================

/// Read files from disk as requested by [`ToolEffect::ReadFiles`].
///
/// Returns one [`tools::FileReadResult`] per input file.
async fn read_files(reads: &[tools::FileRead]) -> Vec<tools::FileReadResult> {
    let mut results = Vec::with_capacity(reads.len());
    for read in reads {
        match tokio::fs::read(&read.path).await {
            Ok(data) => {
                results.push(tools::FileReadResult::Success {
                    path: read.path.clone(),
                    data,
                });
            }
            Err(e) => {
                results.push(tools::FileReadResult::Error {
                    path: read.path.clone(),
                    message: format!("failed to read file: {e}"),
                });
            }
        }
    }
    results
}

// ============================================================================
// API Request Execution
// ============================================================================

/// Result of executing a raw API request: HTTP status code, headers, and bytes.
#[derive(Debug)]
struct RawResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn raw_response_from_api_response(response: ApiResponse) -> RawResponse {
    RawResponse {
        status: response.status.as_u16(),
        headers: header_map_to_pairs(&response.headers),
        body: response.body.bytes,
    }
}

fn header_map_to_pairs(headers: &http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn resume_with_raw_response(
    continuation: tools::Continuation,
    raw: &RawResponse,
) -> (ToolEffect, Vec<SideEffect>) {
    tools::resume(
        continuation,
        IoResult::ApiResponse {
            status: raw.status,
            headers: &raw.headers,
            body: &raw.body,
        },
    )
}

/// Error response when credentials are not configured.
fn not_configured_error() -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::Content::text(
        "Cannot execute API call: credentials are not configured. Configure \
         Onshape API keys (access_key + secret_key), or configure your own OAuth \
         client_id + client_secret and run `onshape-mcp auth login`. To use an \
         OAuth proxy, self-host one and supply its URL explicitly.",
    )])
}

/// Error response when OAuth is pending (client creds present but no tokens).
fn oauth_pending_error() -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::Content::text(
        "Cannot execute API call: OAuth authorization is not complete. Run \
         `onshape-mcp auth login` for direct OAuth, or run it with \
         `--proxy-url <self-hosted-proxy-url>` for an explicitly self-hosted \
         proxy. The server will automatically detect newly written tokens.",
    )])
}

/// Execute a raw API request, returning the HTTP status, headers, and body bytes.
///
/// Handles authentication (Basic or OAuth with proactive/reactive refresh)
/// but does not process the response into a `CallToolResult`.
/// For OAuth, also handles permanent refresh failures by transitioning to
/// `OAuthPending` state.
///
/// The caller must handle states where no API call can be made (`NotConfigured`,
/// `OAuthPending`) before calling this function.
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
        // HTTP OAuth: per-user refresh via the shared OAuthServerState.
        ApiState::HttpOAuth(_) => execute_http_oauth_raw(state, api_req).await,
    }
}

/// Execute a raw request with Basic auth.
async fn execute_basic_raw(
    client: &OnshapeClient,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<RawResponse, McpError> {
    match client.execute(api_req).await {
        Ok(response) => Ok(raw_response_from_api_response(response)),
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

fn permanent_refresh_message(error: &RefreshError) -> String {
    format!(
        "OAuth refresh token is expired or revoked ({error}). Re-authenticate \
         with direct OAuth by running `onshape-mcp auth login`, or explicitly \
         use a self-hosted proxy with \
         `--proxy-url <self-hosted-proxy-url>`. The server will automatically \
         detect newly written tokens."
    )
}

fn oauth_execute_result_from_api_response(response: ApiResponse) -> OAuthExecuteResult {
    OAuthExecuteResult::Ok(raw_response_from_api_response(response))
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
            .post_execute_action(response.status.as_u16(), refreshed)
            == PostExecuteAction::RefreshAndRetry
    {
        if let Err(e) = try_refresh(oauth).await {
            if matches!(e, RefreshError::PermanentExchange(_)) {
                return OAuthExecuteResult::PermanentRefreshFailure {
                    message: permanent_refresh_message(&e),
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
            Ok(response) => oauth_execute_result_from_api_response(response),
            Err(e) => OAuthExecuteResult::Err(McpError::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("HTTP request failed on retry: {e}"),
                None,
            )),
        };
    }

    match result {
        Ok(response) => oauth_execute_result_from_api_response(response),
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
            let pending_method = match oauth.refresh_method {
                RefreshMethod::Direct {
                    client_id,
                    client_secret,
                    ..
                } => PendingRefreshMethod::Direct {
                    client_id,
                    client_secret,
                },
                RefreshMethod::Proxy { proxy_url } => PendingRefreshMethod::Proxy { proxy_url },
            };
            *state = ApiState::OAuthPending(Box::new(OAuthPendingState {
                refresh_method: pending_method,
                base_url: oauth.base_url,
                timeout: oauth.timeout,
                token_path: oauth.token_path,
            }));
            // Return the error as a raw 401 response so the caller can
            // process it appropriately.
            Ok(RawResponse {
                status: 401,
                headers: vec![],
                body: message.into_bytes(),
            })
        }
    }
}

/// Execute a raw request with per-user HTTP OAuth, including proactive and
/// reactive refresh.
///
/// Follows the same proactive/reactive pattern as [`execute_oauth_inner`] but
/// delegates token refresh to the shared [`OAuthServerState`] which holds the
/// server's Onshape client credentials and per-user token storage.
///
/// On permanent refresh failure, returns a synthetic 401 with a re-auth message
/// (there is no state transition since HTTP OAuth state is per-request).
async fn execute_http_oauth_raw(
    state: &mut ApiState,
    api_req: &onshape_client_core::request::ApiRequest,
) -> Result<RawResponse, McpError> {
    let ApiState::HttpOAuth(http_oauth) = state else {
        return Err(McpError::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "expected HttpOAuth state",
            None,
        ));
    };

    // Proactive refresh: check if the token is expiring soon.
    let refresh_margin = chrono::Duration::seconds(REFRESH_MARGIN_SECS);
    let now = chrono::Utc::now();
    let needs_proactive_refresh = http_oauth
        .expires_at
        .is_some_and(|exp| exp - refresh_margin <= now);

    let mut refreshed = false;
    if needs_proactive_refresh {
        match http_oauth
            .oauth_state
            .refresh_user_onshape_tokens(&http_oauth.user_id, http_oauth.expires_at)
            .await
        {
            Ok(new_tokens) => {
                rebuild_http_oauth_client(http_oauth, &new_tokens)?;
                refreshed = true;
            }
            Err(e) => {
                eprintln!(
                    "[oauth] proactive refresh failed for user {}: {e}",
                    http_oauth.user_id
                );
                // Continue with the current token — it might still work.
            }
        }
    }

    // Execute the API call.
    let result = http_oauth.client.execute(api_req).await;

    // Reactive: retry on 401 if we haven't already refreshed.
    if let Ok(ref response) = result
        && response.status == 401
        && !refreshed
    {
        match http_oauth
            .oauth_state
            .refresh_user_onshape_tokens(&http_oauth.user_id, http_oauth.expires_at)
            .await
        {
            Ok(new_tokens) => {
                rebuild_http_oauth_client(http_oauth, &new_tokens)?;
                // Retry the request with the refreshed token.
                let retry = http_oauth.client.execute(api_req).await;
                return match retry {
                    Ok(response) => Ok(raw_response_from_api_response(response)),
                    Err(e) => Err(McpError::new(
                        rmcp::model::ErrorCode::INTERNAL_ERROR,
                        format!("HTTP request failed on retry: {e}"),
                        None,
                    )),
                };
            }
            Err(oauth_server::UserTokenRefreshError::PermanentExchange(msg)) => {
                return Ok(RawResponse {
                    status: 401,
                    headers: vec![],
                    body: format!(
                        "Onshape token refresh permanently failed ({msg}). \
                         Reconnect to this self-hosted HTTP server and complete \
                         its browser OAuth flow again."
                    )
                    .into_bytes(),
                });
            }
            Err(e) => {
                return Err(McpError::new(
                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                    format!("API returned 401 and token refresh failed: {e}"),
                    None,
                ));
            }
        }
    }

    match result {
        Ok(response) => Ok(raw_response_from_api_response(response)),
        Err(e) => Err(McpError::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("HTTP request failed: {e}"),
            None,
        )),
    }
}

/// Rebuild the HTTP OAuth client with refreshed tokens.
fn rebuild_http_oauth_client(
    http_oauth: &mut HttpOAuthApiState,
    new_tokens: &oauth_server::UserOnshapeTokens,
) -> Result<(), McpError> {
    http_oauth.client = OnshapeClient::new(ClientConfig {
        base_url: http_oauth.base_url.clone(),
        auth: ClientAuthConfig::Bearer {
            access_token: AccessToken::new(new_tokens.access_token().expose_secret().to_string()),
        },
        timeout: Some(http_oauth.timeout),
    })
    .map_err(|e| {
        McpError::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("failed to rebuild per-user API client after refresh: {e}"),
            None,
        )
    })?;
    http_oauth.expires_at = new_tokens.expires_at();
    Ok(())
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
// OAuth Login Flow Handler
// ============================================================================

/// Handle the `OAuthLoginFlow` tool result by starting a login flow.
///
/// Any previous login flow is cancelled first (callback servers shut down,
/// background task aborted) to free the callback port before starting the
/// new flow.
///
/// Extracted from `call_tool` to keep that function within clippy's line limit.
async fn handle_oauth_login_flow(
    mode: onshape_mcp_core::tools::LoginMode,
    login_state: &tokio::sync::Mutex<login::LoginState>,
) -> Result<CallToolResult, McpError> {
    let mut login = login_state.lock().await;

    // Cancel any existing flow first — this shuts down the old callback
    // server and frees port 18338 so the new flow can bind it.
    login.clear();

    // Start the login flow.
    match login::start_login_flow(&mode).await {
        Ok(handle) => {
            let authorize_url = handle.authorize_url.clone();

            // Store the new session.
            login.set_active(handle.session);
            drop(login);

            Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                format!(
                    "The OAuth authorization flow has started. You MUST present the \
                 following URL to the user and instruct them to open it in their \
                 browser to authorize:\n\n{authorize_url}\n\n\
                 After they authorize in the browser, the server will automatically \
                 detect the new tokens via the local callback.",
                ),
            )]))
        }
        Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(
            format!("Failed to start login flow: {e}"),
        )])),
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
    /// The token file changed but could not safely replace active credentials.
    #[error("token file changed while waiting for the credential lock: {0}")]
    ExternalTokenChange(String),
}

/// Check if a refresh error indicates a permanently dead refresh token.
///
/// OAuth error codes `unauthorized_client` and `invalid_grant` mean the
/// refresh token is revoked or expired — the user must re-authenticate.
pub(crate) fn is_permanent_refresh_failure(error_message: &str) -> bool {
    let lower = error_message.to_lowercase();
    lower.contains("unauthorized_client") || lower.contains("invalid_grant")
}

/// Attempt to refresh the OAuth access token.
///
/// The cross-process token lock covers the complete transaction: definitive
/// token-file reread, refresh-token consumption, exchange, and publication.
async fn try_refresh(oauth: &mut OAuthApiState) -> Result<(), RefreshError> {
    let token_lock = crate::oauth::TokenFileLock::acquire(&oauth.token_path).await?;
    let current_snapshot = TokenFileSnapshot::capture(&oauth.token_path);
    if current_snapshot != oauth.last_observed_token_snapshot {
        let token_file = crate::oauth::load_token_file(&oauth.token_path).map_err(|error| {
            RefreshError::ExternalTokenChange(format!(
                "the changed file is missing, unreadable, or invalid; refusing to overwrite it: {error}"
            ))
        })?;
        if !adopt_external_token_file(
            oauth,
            &token_file,
            ExternalTokenAdoptionPolicy::WatcherObservedWrite,
        )? {
            return Err(RefreshError::ExternalTokenChange(
                "the changed file does not contain complete tokens and refresh metadata; refusing to overwrite it"
                    .to_string(),
            ));
        }
        oauth.last_observed_token_snapshot = current_snapshot;
        return Ok(());
    }

    // We use an enum discriminant check + separate blocks to avoid borrowing
    // `oauth` (mutable) and `oauth.refresh_method` (immutable) simultaneously.
    let is_proxy = matches!(&oauth.refresh_method, RefreshMethod::Proxy { .. });
    if is_proxy {
        // Extract the proxy URL (cheap clone of a String).
        let proxy_url = match &oauth.refresh_method {
            RefreshMethod::Proxy { proxy_url } => onshape_mcp_core::validate_proxy_url(proxy_url)
                .map_err(|message| {
                RefreshError::Exchange(format!("invalid proxy refresh URL: {message}"))
            })?,
            RefreshMethod::Direct { .. } => unreachable!(),
        };
        try_refresh_proxy(oauth, &proxy_url).await?;
    } else {
        try_refresh_direct(oauth).await?;
    }

    let token_file = oauth
        .token_metadata
        .with_tokens(oauth.session.tokens.clone());
    crate::oauth::save_token_file_locked(&oauth.token_path, &token_file, &token_lock)?;
    oauth.last_observed_token_snapshot = TokenFileSnapshot::capture(&oauth.token_path);
    oauth.rebuild_client()?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TokenFileSnapshot {
    Readable {
        contents: Vec<u8>,
        identity: TokenFileIdentity,
    },
    Missing,
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TokenFileIdentity {
    length: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl TokenFileSnapshot {
    fn from_read(contents: Vec<u8>, metadata: &std::fs::Metadata) -> Self {
        Self::Readable {
            contents,
            identity: TokenFileIdentity {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                created: metadata.created().ok(),
                #[cfg(unix)]
                device: metadata.dev(),
                #[cfg(unix)]
                inode: metadata.ino(),
            },
        }
    }

    pub(crate) fn capture(path: &std::path::Path) -> Self {
        match crate::oauth::load_token_file_with_snapshot(path) {
            Ok((_, snapshot)) => snapshot,
            Err(
                crate::oauth::TokenFileError::Read { ref source, .. }
                | crate::oauth::TokenFileError::MetadataError { ref source, .. },
            ) if source.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            Err(_) => Self::Unreadable,
        }
    }
}

/// Direct refresh: exchange refresh token with Onshape via the `oauth2` crate.
async fn try_refresh_direct(oauth: &mut OAuthApiState) -> Result<(), RefreshError> {
    let RefreshMethod::Direct { oauth_client, .. } = &oauth.refresh_method else {
        unreachable!("try_refresh_direct called with non-Direct method");
    };

    // Clone the refresh token to avoid holding a borrow across `.await`.
    let refresh_token = oauth.session.refresh_token().clone();

    let response = oauth_client
        .exchange_refresh_token(&refresh_token)
        .request_async(&ReqwestClient::from(oauth.refresh_http.clone()))
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if is_permanent_refresh_failure(&msg) {
                RefreshError::PermanentExchange(msg)
            } else {
                RefreshError::Exchange(msg)
            }
        })?;

    oauth.session.apply_refresh(&response, chrono::Utc::now());
    Ok(())
}

/// Proxy refresh: POST `{ "refresh_token": "..." }` to the proxy.
///
/// The proxy adds client credentials and forwards to Onshape, returning
/// the token response as-is.
///
/// If the proxy returns 403 and reports an IPv6 `source_ip`, the request
/// is automatically retried with a client that forces IPv4 connections.
/// This handles the common case where the proxy's `ALLOWED_SOURCES` only
/// resolves to IPv4 addresses but the client connects via IPv6.
async fn try_refresh_proxy(oauth: &mut OAuthApiState, proxy_url: &str) -> Result<(), RefreshError> {
    let url = format!("{}/token/refresh", proxy_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "refresh_token": oauth.session.refresh_token().secret(),
    });

    // First attempt with the default client.
    let (status, response_body) = send_proxy_request(&oauth.refresh_http, &url, &body).await?;

    // If the proxy rejected us with 403 and reports an IPv6 source IP,
    // retry with a client that forces IPv4 connections.
    let (status, response_body) = if status == reqwest::StatusCode::FORBIDDEN {
        match build_ipv4_retry_client(&response_body) {
            Some(ipv4_client) => send_proxy_request(&ipv4_client, &url, &body).await?,
            None => (status, response_body),
        }
    } else {
        (status, response_body)
    };

    if !status.is_success() {
        let msg = format!("proxy returned {status}: {response_body}");
        if is_permanent_refresh_failure(&msg) {
            return Err(RefreshError::PermanentExchange(msg));
        }
        return Err(RefreshError::Exchange(msg));
    }

    // Parse the proxy response (same shape as Onshape's token response).
    let token_response: ProxyTokenResponse = serde_json::from_str(&response_body)
        .map_err(|e| RefreshError::Exchange(format!("failed to parse proxy response: {e}")))?;

    let now = chrono::Utc::now();
    let expires_at = token_response
        .expires_in
        .and_then(chrono::Duration::try_seconds)
        .map(|d| now + d);

    let refresh_token = token_response
        .refresh_token
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| oauth.session.refresh_token().secret().clone());
    let new_tokens = onshape_client_core::oauth::OAuthTokenData::from_raw(
        token_response.access_token,
        refresh_token,
        expires_at,
        token_response.token_type.unwrap_or_else(|| "bearer".into()),
        token_response
            .scope
            .map(|s| s.split(' ').map(String::from).collect()),
    );
    oauth.session.tokens = new_tokens;
    Ok(())
}

/// Send a POST request to the proxy and return the status + body text.
async fn send_proxy_request(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<(reqwest::StatusCode, String), RefreshError> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| RefreshError::Exchange(e.to_string()))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| RefreshError::Exchange(format!("failed to read proxy response: {e}")))?;

    Ok((status, text))
}

/// If a 403 response contains an IPv6 `source_ip`, build a `reqwest::Client`
/// that forces IPv4 connections for a retry attempt.
///
/// Returns `None` if the response cannot be parsed, the `source_ip` is IPv4,
/// or the IPv4 client cannot be constructed.
fn build_ipv4_retry_client(response_body: &str) -> Option<reqwest::Client> {
    let parsed: ProxyForbiddenResponse = serde_json::from_str(response_body).ok()?;

    // IPv6 addresses always contain a colon; IPv4 never does.
    if !parsed.source_ip.contains(':') {
        return None;
    }

    reqwest::Client::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .timeout(Duration::from_secs(30))
        .build()
        .ok()
}

/// Deserialization target for the proxy's 403 "forbidden" response.
///
/// The proxy includes the connecting IP so the client can detect IPv6
/// and retry with forced IPv4.
#[derive(serde::Deserialize)]
struct ProxyForbiddenResponse {
    /// The IP address the proxy saw for the incoming request.
    source_ip: String,
}

/// Deserialization target for the proxy/Onshape token response JSON.
#[derive(serde::Deserialize)]
struct ProxyTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
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
            expires_at: data.tokens.expires_at,
            proxy_url: data.proxy_url,
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
    if let Some(proxy_url) = config.auth.proxy_url.as_deref() {
        onshape_mcp_core::validate_proxy_url(proxy_url)
            .map_err(|message| format!("invalid configured auth.proxy_url: {message}"))?;
    }
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
                timeout: Some(config.api.timeout),
            })?;
            Ok(ApiState::Basic(client))
        }
        ResolvedAuth::OAuthReady { .. } => {
            build_oauth_ready_state(config, server_url, token_path.as_deref())
        }
        ResolvedAuth::OAuthPending => build_oauth_pending_state(config, server_url, token_path),
        ResolvedAuth::NotConfigured {
            configured_method,
            detail,
        } => Ok(ApiState::NotConfigured {
            configured_method,
            detail,
        }),
    }
}

/// Determine the refresh method from config and token file data.
///
/// Priority:
/// 1. Config `proxy_url`
/// 2. Complete config `client_id` + `client_secret`
/// 3. Token-file refresh metadata
fn determine_refresh_method(
    config: &AppConfig,
    token_file: Option<&crate::oauth::McpOAuthTokenFile>,
) -> Result<Option<RefreshMethod>, String> {
    if let Some(url) = config.auth.proxy_url.as_deref() {
        return onshape_mcp_core::validate_proxy_url(url)
            .map(|proxy_url| Some(RefreshMethod::Proxy { proxy_url }));
    }

    // Direct mode: need both client_id and client_secret.
    if let (Some(client_id), Some(client_secret)) =
        (&config.auth.client_id, &config.auth.client_secret)
        && !config.auth.direct_credentials_from_token_file
    {
        let oauth_client = onshape_oauth_client(client_id, client_secret.expose_secret());
        return Ok(Some(RefreshMethod::Direct {
            oauth_client: Box::new(oauth_client),
            client_id: client_id.clone(),
            client_secret: SecretString::from(client_secret.clone()),
        }));
    }

    token_file.map_or(Ok(None), refresh_method_from_token_file)
}

/// Determine the pending refresh method from config.
fn determine_pending_refresh_method(
    config: &AppConfig,
) -> Result<Option<PendingRefreshMethod>, String> {
    // Proxy mode.
    if let Some(proxy_url) = config.auth.proxy_url.as_ref() {
        return onshape_mcp_core::validate_proxy_url(proxy_url)
            .map(|proxy_url| Some(PendingRefreshMethod::Proxy { proxy_url }));
    }

    // Direct mode.
    if let (Some(client_id), Some(client_secret)) =
        (&config.auth.client_id, &config.auth.client_secret)
    {
        return Ok(Some(PendingRefreshMethod::Direct {
            client_id: client_id.clone(),
            client_secret: SecretString::from(client_secret.clone()),
        }));
    }

    Ok(None)
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
    let Some(token_path) = token_path else {
        unreachable!("OAuthReady but token path is None");
    };

    let (token_file, last_observed_token_snapshot) =
        crate::oauth::load_token_file_with_snapshot(token_path)
            .map_err(|e| format!("failed to load token file: {e}"))?;

    let refresh_method = determine_refresh_method(config, Some(&token_file))?
        .ok_or("OAuthReady but no refresh method available")?;

    let token_metadata = refresh_method.token_metadata_from_file(&token_file);
    let session = OAuthSession::new(
        token_file.tokens,
        chrono::Duration::seconds(REFRESH_MARGIN_SECS),
    );

    let timeout = config.api.timeout;
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
        token_metadata,
        refresh_method,
        client,
        refresh_http,
        token_path: token_path.to_path_buf(),
        last_observed_token_snapshot,
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
) -> Result<ApiState, Box<dyn std::error::Error + Send + Sync>> {
    let Some(token_path) = token_path else {
        // Can't watch for tokens without a path — fall back to NotConfigured
        return Ok(ApiState::NotConfigured {
            configured_method: config.auth.method,
            detail: "OAuth pending but no token file path available".into(),
        });
    };

    let Some(refresh_method) = determine_pending_refresh_method(config)? else {
        unreachable!("OAuthPending but no refresh method available");
    };

    Ok(ApiState::OAuthPending(Box::new(OAuthPendingState {
        refresh_method,
        base_url: server_url.to_string(),
        timeout: config.api.timeout,
        token_path,
    })))
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
            ApiState::Basic(_) | ApiState::HttpOAuth(_) => None,
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
            timeout: server.config.api.timeout,
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
// HTTP Transport Entry Point
// ============================================================================

/// Runs the MCP server over Streamable HTTP transport.
///
/// Serves both the OAuth endpoints (metadata, DCR, authorize, callback, token)
/// and the MCP endpoint at `/mcp` (protected by bearer token auth).
///
/// # Arguments
///
/// * `name` - The server name (typically from `CARGO_PKG_NAME`)
/// * `version` - The server version (typically from `CARGO_PKG_VERSION`)
/// * `config` - Application configuration (loaded by the binary crate)
///
/// # Errors
///
/// Returns an error if required config fields are missing, or if the server
/// fails to start.
#[allow(clippy::too_many_lines)]
pub async fn run_http(
    name: &str,
    version: &str,
    config: AppConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::sync::Arc;

    use axum::{Router, middleware};
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use tokio_util::sync::CancellationToken;

    // Validate required config.
    let public_url = config
        .http
        .public_url
        .clone()
        .ok_or("http.public_url is required for the HTTP transport")?;
    let parsed_public_url = url::Url::parse(&public_url)
        .map_err(|e| format!("http.public_url must be a valid absolute URL: {e}"))?;
    if !matches!(parsed_public_url.scheme(), "http" | "https") {
        return Err("http.public_url must use http:// or https://".into());
    }
    if !parsed_public_url.has_host() {
        return Err("http.public_url must include a host".into());
    }
    if parsed_public_url.query().is_some() || parsed_public_url.fragment().is_some() {
        return Err("http.public_url must not include query parameters or fragments".into());
    }
    if parsed_public_url.path() != "/" {
        return Err(
            "http.public_url must not include a path (e.g. use 'https://example.com' \
             not 'https://example.com/prefix') — the server mounts all endpoints at the root"
                .into(),
        );
    }
    // Strip trailing slash from the path for consistent path extension via
    // Url::path_segments_mut().extend().
    let public_url = {
        let mut url = parsed_public_url;
        let trimmed = url.path().trim_end_matches('/').to_string();
        url.set_path(&trimmed);
        url
    };
    let onshape_client_id = config
        .http
        .onshape_client_id
        .clone()
        .ok_or("http.onshape_client_id is required for the HTTP transport")?;
    let onshape_client_secret = config
        .http
        .onshape_client_secret
        .clone()
        .ok_or("http.onshape_client_secret is required for the HTTP transport")?;

    let host = config.http.host.clone();
    let port = config.http.port;

    let allowed_user_ids: Vec<String> = config
        .http
        .allowed_users
        .iter()
        .map(|u| u.id.clone())
        .collect();

    if allowed_user_ids.is_empty() {
        eprintln!(
            "WARNING: allowed_users is empty — all users will be denied access. \
             Configure allowed_users in the config file or via --allowed-users."
        );
    }

    // Build shared state.
    let spec = OpenApiSpec::from_json_with_server_url_fallback(
        OPENAPI_SPEC_JSON,
        OPENAPI_SERVER_URL_FALLBACK,
    )?;
    let info = onshape_mcp_core::server_info(name, version);
    let config = Arc::new(config);
    let spec = Arc::new(spec);
    // Build the OAuth server state.
    let oauth_state = Arc::new(oauth_server::OAuthServerState::new(
        public_url.clone(),
        onshape_client_id,
        onshape_client_secret,
        allowed_user_ids,
    ));

    // Build the MCP service factory.
    //
    // Each session gets a fresh `OnshapeMcpServer` instance with its own
    // `ValidationState`. In HTTP mode, per-user credentials come from the
    // `UserContext` in the request extensions (set by auth middleware),
    // not from the shared `api_state`.
    let api_state = Arc::new(tokio::sync::Mutex::new(ApiState::NotConfigured {
        configured_method: onshape_client_core::auth::AuthMethod::OAuth,
        detail: "HTTP mode: per-user credentials via OAuth".to_string(),
    }));

    let cancellation_token = CancellationToken::new();

    let mcp_config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancellation_token.clone());

    let factory_info = info.clone();
    let factory_config = Arc::clone(&config);
    let factory_spec = Arc::clone(&spec);
    let factory_api_state = Arc::clone(&api_state);
    let factory_oauth_state = Arc::clone(&oauth_state);

    let mcp_service = StreamableHttpService::new(
        move || {
            // Each session gets its own ValidationState so that one user's
            // API response status does not overwrite another user's view.
            let per_session_validation =
                Arc::new(tokio::sync::Mutex::new(ValidationState::default()));
            Ok(OnshapeMcpServer::from_shared_state(
                factory_info.clone(),
                Arc::clone(&factory_config),
                Arc::clone(&factory_spec),
                Arc::clone(&factory_api_state),
                per_session_validation,
                Some(Arc::clone(&factory_oauth_state)),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );

    // Build the MCP endpoint with auth middleware.
    let oauth_state_for_middleware = Arc::clone(&oauth_state);
    let mcp_router =
        Router::new()
            .nest_service("/mcp", mcp_service)
            .layer(middleware::from_fn_with_state(
                oauth_state_for_middleware,
                oauth_server::auth_middleware,
            ));

    // Build the full app: OAuth routes + protected MCP route.
    let app = oauth_server::oauth_router(oauth_state).merge(mcp_router);

    // Bind and serve. Bracket IPv6 hosts to produce valid socket addresses.
    // Normalize by stripping any existing brackets so we don't double-bracket.
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    let bind_addr = if normalized_host.contains(':') {
        format!("[{normalized_host}]:{port}")
    } else {
        format!("{normalized_host}:{port}")
    };
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let mcp_endpoint = {
        let mut url = public_url.clone();
        url.path_segments_mut()
            .map_err(|()| "public_url cannot be a base URL")?
            .push("mcp");
        url
    };
    eprintln!("HTTP transport listening on {bind_addr}");
    eprintln!("Public URL: {public_url}");
    eprintln!("MCP endpoint: {mcp_endpoint}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            // Ignore errors from ctrl_c — if we can't install the handler,
            // we simply won't have graceful shutdown on Ctrl+C.
            let _ = tokio::signal::ctrl_c().await;
            cancellation_token.cancel();
        })
        .await?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use oauth2::{AccessToken, RefreshToken};
    use onshape_client_core::request::ResponseBody;

    fn first_text(result: &CallToolResult) -> &str {
        result.content[0]
            .raw
            .as_text()
            .expect("should contain text")
            .text
            .as_str()
    }

    #[test]
    fn not_configured_error_is_client_neutral_and_actionable() {
        let result = not_configured_error();
        let message = first_text(&result);

        assert!(message.contains("access_key + secret_key"));
        assert!(message.contains("onshape-mcp auth login"));
        assert!(message.contains("self-host"));
        assert!(!message.contains("OpenCode"));
    }

    #[test]
    fn oauth_pending_error_is_client_neutral_and_actionable() {
        let result = oauth_pending_error();
        let message = first_text(&result);

        assert!(message.contains("onshape-mcp auth login"));
        assert!(message.contains("--proxy-url"));
        assert!(!message.contains("editor"));
    }

    #[test]
    fn permanent_refresh_error_is_client_neutral_and_actionable() {
        let message = permanent_refresh_message(&RefreshError::PermanentExchange(
            "invalid_grant".to_string(),
        ));

        assert!(message.contains("onshape-mcp auth login"));
        assert!(message.contains("--proxy-url"));
        assert!(!message.contains("OpenCode"));
    }

    #[test]
    fn blank_proxy_metadata_does_not_override_direct_credentials() {
        let mut config = AppConfig::default();
        config.auth.client_id = Some("direct-id".to_string());
        config.auth.client_secret = Some(SecretString::from("direct-secret"));
        config.auth.proxy_url = None;
        let mut token_file = token_file_with_all_metadata();
        token_file.proxy_url = Some(String::new());

        assert!(matches!(
            determine_refresh_method(&config, Some(&token_file)),
            Ok(Some(RefreshMethod::Direct { .. }))
        ));
        assert!(matches!(
            determine_pending_refresh_method(&config),
            Ok(Some(PendingRefreshMethod::Direct { .. }))
        ));
    }

    #[test]
    fn startup_refresh_method_uses_explicit_config_before_persisted_fallback() {
        let mut config = AppConfig::default();
        config.auth.client_id = Some("explicit-direct".to_string());
        config.auth.client_secret = Some(SecretString::from("explicit-secret"));
        let persisted_proxy = McpOAuthTokenFile {
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some("https://persisted-proxy.example.com".to_string()),
            ..token_file_with_all_metadata()
        };
        assert!(matches!(
            determine_refresh_method(&config, Some(&persisted_proxy)),
            Ok(Some(RefreshMethod::Direct { ref client_id, .. })) if client_id == "explicit-direct"
        ));

        config.auth.proxy_url = Some("https://explicit-proxy.example.com".to_string());
        let persisted_direct = McpOAuthTokenFile {
            client_id: Some("persisted-direct".to_string()),
            client_secret: Some("persisted-secret".to_string()),
            proxy_url: None,
            ..token_file_with_all_metadata()
        };
        assert!(matches!(
            determine_refresh_method(&config, Some(&persisted_direct)),
            Ok(Some(RefreshMethod::Proxy { ref proxy_url })) if proxy_url == "https://explicit-proxy.example.com"
        ));

        let invalid_persisted_proxy = McpOAuthTokenFile {
            client_id: None,
            client_secret: None,
            proxy_url: Some("http://invalid-persisted.example.com".to_string()),
            ..token_file_with_all_metadata()
        };
        assert!(matches!(
            determine_refresh_method(&config, Some(&invalid_persisted_proxy)),
            Ok(Some(RefreshMethod::Proxy { ref proxy_url })) if proxy_url == "https://explicit-proxy.example.com"
        ));

        let conflicting_persisted = token_file_with_all_metadata();
        assert!(matches!(
            determine_refresh_method(&config, Some(&conflicting_persisted)),
            Ok(Some(RefreshMethod::Proxy { ref proxy_url })) if proxy_url == "https://explicit-proxy.example.com"
        ));

        config.auth.proxy_url = None;
        assert!(matches!(
            determine_refresh_method(&config, Some(&invalid_persisted_proxy)),
            Ok(Some(RefreshMethod::Direct { ref client_id, .. })) if client_id == "explicit-direct"
        ));
        assert!(matches!(
            determine_refresh_method(&config, Some(&conflicting_persisted)),
            Ok(Some(RefreshMethod::Direct { ref client_id, .. })) if client_id == "explicit-direct"
        ));

        config.auth.client_id = None;
        config.auth.client_secret = None;
        assert!(matches!(
            determine_refresh_method(&config, Some(&persisted_proxy)),
            Ok(Some(RefreshMethod::Proxy { .. }))
        ));
        assert!(matches!(
            determine_refresh_method(&config, Some(&persisted_direct)),
            Ok(Some(RefreshMethod::Direct { ref client_id, .. })) if client_id == "persisted-direct"
        ));

        config.auth.client_id = Some("previous-persisted".to_string());
        config.auth.client_secret = Some(SecretString::from("previous-secret"));
        config.auth.direct_credentials_from_token_file = true;
        assert!(matches!(
            determine_refresh_method(&config, Some(&persisted_direct)),
            Ok(Some(RefreshMethod::Direct { ref client_id, .. })) if client_id == "persisted-direct"
        ));
    }

    // --- raw response conversion tests ---

    #[test]
    fn raw_response_from_api_response_preserves_bytes_and_headers() {
        let response = ApiResponse {
            status: http::StatusCode::OK,
            headers: http::HeaderMap::from_iter([(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/plain"),
            )]),
            body: ResponseBody::from("ok"),
        };

        let raw = raw_response_from_api_response(response);

        assert_eq!(raw.status, 200);
        assert_eq!(
            raw.headers,
            vec![("content-type".to_string(), "text/plain".to_string())]
        );
        assert_eq!(raw.body, b"ok");
    }

    #[test]
    fn resume_with_raw_response_passes_invalid_utf8_to_continuation() {
        let response = ApiResponse {
            status: http::StatusCode::OK,
            headers: http::HeaderMap::from_iter([(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            )]),
            body: ResponseBody::from(vec![0xff]),
        };

        let raw = raw_response_from_api_response(response);
        let (tool_effect, side_effects) =
            resume_with_raw_response(tools::Continuation::FormatApiResponse, &raw);

        assert!(side_effects.is_empty());
        let tools::ToolEffect::Done(Ok(result)) = tool_effect else {
            panic!("should produce a done result");
        };
        assert_eq!(result.is_error, Some(false));
        let text = result.content[0]
            .raw
            .as_text()
            .expect("should be text content");
        let value: serde_json::Value = serde_json::from_str(&text.text)
            .expect("binary response metadata should be valid JSON");
        assert_eq!(value["encoding"], "base64");
        assert_eq!(value["body"], "/w==");
    }

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

    fn token_file_with_all_metadata() -> McpOAuthTokenFile {
        McpOAuthTokenFile {
            tokens: onshape_client_core::oauth::OAuthTokenData {
                access_token: AccessToken::new("access".to_string()),
                refresh_token: RefreshToken::new("refresh".to_string()),
                expires_at: None,
                token_type: "bearer".to_string(),
                scopes: None,
            },
            client_id: Some("client-id".to_string()),
            client_secret: Some("client-secret".to_string()),
            proxy_url: Some("https://proxy.example.com".to_string()),
        }
    }

    fn token_data(
        access_token: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> onshape_client_core::oauth::OAuthTokenData {
        onshape_client_core::oauth::OAuthTokenData {
            access_token: AccessToken::new(access_token.to_string()),
            refresh_token: RefreshToken::new(format!("refresh-{access_token}")),
            expires_at: Some(expires_at),
            token_type: "bearer".to_string(),
            scopes: None,
        }
    }

    fn oauth_state(refresh_method: RefreshMethod) -> OAuthApiState {
        let tokens = token_data("current", chrono::Utc::now() + chrono::Duration::minutes(1));
        let session = OAuthSession::new(tokens, chrono::Duration::seconds(REFRESH_MARGIN_SECS));
        let base_url = "https://cad.onshape.com/api/v6".to_string();
        let timeout = Duration::from_secs(30);
        let client = OnshapeClient::new(ClientConfig {
            base_url: base_url.clone(),
            auth: ClientAuthConfig::Bearer {
                access_token: AccessToken::new(session.access_token().secret().clone()),
            },
            timeout: Some(timeout),
        })
        .expect("should build test client");
        let refresh_http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("should build refresh client");

        OAuthApiState {
            session,
            token_metadata: McpOAuthTokenMetadata {
                client_id: None,
                client_secret: None,
                proxy_url: None,
            },
            refresh_method,
            client,
            refresh_http,
            token_path: PathBuf::from("tokens.json"),
            last_observed_token_snapshot: TokenFileSnapshot::Missing,
            base_url,
            timeout,
        }
    }

    #[test]
    fn direct_refresh_metadata_drops_proxy_url() {
        let method = RefreshMethod::Direct {
            oauth_client: Box::new(onshape_oauth_client("client-id", "client-secret")),
            client_id: "client-id".to_string(),
            client_secret: SecretString::from("client-secret".to_string()),
        };

        let metadata = method.token_metadata_from_file(&token_file_with_all_metadata());

        assert_eq!(metadata.client_id.as_deref(), Some("client-id"));
        assert_eq!(metadata.client_secret.as_deref(), Some("client-secret"));
        assert!(metadata.proxy_url.is_none());
    }

    #[test]
    fn proxy_refresh_metadata_drops_client_secret() {
        let method = RefreshMethod::Proxy {
            proxy_url: "https://proxy.example.com".to_string(),
        };

        let metadata = method.token_metadata_from_file(&token_file_with_all_metadata());

        assert_eq!(metadata.client_id.as_deref(), Some("client-id"));
        assert!(metadata.client_secret.is_none());
        assert_eq!(
            metadata.proxy_url.as_deref(),
            Some("https://proxy.example.com")
        );
    }

    #[test]
    fn watcher_adoption_switches_direct_to_proxy_with_equal_or_missing_expiry() {
        for missing_expiry in [false, true] {
            let mut oauth = oauth_state(RefreshMethod::Direct {
                oauth_client: Box::new(onshape_oauth_client("old-client", "old-secret")),
                client_id: "old-client".to_string(),
                client_secret: SecretString::from("old-secret".to_string()),
            });
            let expires_at = oauth.session.tokens.expires_at.expect("should expire");
            let mut token_file = McpOAuthTokenFile {
                tokens: token_data("proxy-external", expires_at),
                client_id: Some("proxy-client".to_string()),
                client_secret: None,
                proxy_url: Some("https://proxy.example.com".to_string()),
            };
            if missing_expiry {
                token_file.tokens.expires_at = None;
            }

            let adopted = adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::WatcherObservedWrite,
            )
            .expect("should adopt token file");

            assert!(adopted);
            assert_eq!(oauth.session.access_token().secret(), "proxy-external");
            let RefreshMethod::Proxy { proxy_url } = &oauth.refresh_method else {
                panic!("should switch to proxy refresh");
            };
            assert_eq!(proxy_url, "https://proxy.example.com");
            assert_eq!(
                oauth.token_metadata.client_id.as_deref(),
                Some("proxy-client")
            );
            assert!(oauth.token_metadata.client_secret.is_none());
            assert_eq!(
                oauth.token_metadata.proxy_url.as_deref(),
                Some("https://proxy.example.com")
            );
        }
    }

    #[test]
    fn watcher_adoption_switches_proxy_to_direct_with_equal_or_missing_expiry() {
        for missing_expiry in [false, true] {
            let mut oauth = oauth_state(RefreshMethod::Proxy {
                proxy_url: "https://old-proxy.example.com".to_string(),
            });
            let expires_at = oauth.session.tokens.expires_at.expect("should expire");
            let mut token_file = McpOAuthTokenFile {
                tokens: token_data("direct-external", expires_at),
                client_id: Some("direct-client".to_string()),
                client_secret: Some("direct-secret".to_string()),
                proxy_url: None,
            };
            if missing_expiry {
                token_file.tokens.expires_at = None;
            }

            let adopted = adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::WatcherObservedWrite,
            )
            .expect("should adopt token file");

            assert!(adopted);
            assert_eq!(oauth.session.access_token().secret(), "direct-external");
            assert_eq!(oauth.session.tokens.expires_at.is_none(), missing_expiry);
            let RefreshMethod::Direct { client_id, .. } = &oauth.refresh_method else {
                panic!("should switch to direct refresh");
            };
            assert_eq!(client_id, "direct-client");
            assert_eq!(
                oauth.token_metadata.client_id.as_deref(),
                Some("direct-client")
            );
            assert_eq!(
                oauth.token_metadata.client_secret.as_deref(),
                Some("direct-secret")
            );
            assert!(oauth.token_metadata.proxy_url.is_none());
        }
    }

    #[test]
    fn refresh_read_rejects_opposite_proxy_mode_regardless_of_expiry() {
        let mut oauth = oauth_state(RefreshMethod::Direct {
            oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
            client_id: "current-client".to_string(),
            client_secret: SecretString::from("current-secret".to_string()),
        });
        oauth.token_metadata = McpOAuthTokenMetadata {
            client_id: Some("current-client".to_string()),
            client_secret: Some("current-secret".to_string()),
            proxy_url: None,
        };
        let current_expiry = oauth.session.tokens.expires_at.expect("should expire");

        for expires_at in [
            Some(current_expiry - chrono::Duration::seconds(10)),
            Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
            Some(current_expiry),
            None,
        ] {
            let mut tokens = token_data("proxy-external", current_expiry);
            tokens.expires_at = expires_at;
            let token_file = McpOAuthTokenFile {
                tokens,
                client_id: Some("proxy-client".to_string()),
                client_secret: None,
                proxy_url: Some("https://proxy.example.com".to_string()),
            };

            let adopted = adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::RoutineRefreshRead,
            )
            .expect("should reject opposite-mode token file");

            assert!(!adopted);
            assert_eq!(oauth.session.access_token().secret(), "current");
            assert_eq!(oauth.session.tokens.expires_at, Some(current_expiry));
            let RefreshMethod::Direct {
                client_id,
                client_secret,
                ..
            } = &oauth.refresh_method
            else {
                panic!("should remain direct");
            };
            assert_eq!(client_id, "current-client");
            assert_eq!(client_secret.expose_secret(), "current-secret");
            assert_eq!(
                oauth.token_metadata.client_id.as_deref(),
                Some("current-client")
            );
            assert_eq!(
                oauth.token_metadata.client_secret.as_deref(),
                Some("current-secret")
            );
            assert!(oauth.token_metadata.proxy_url.is_none());
        }
    }

    #[test]
    fn refresh_read_rejects_opposite_direct_mode_regardless_of_expiry() {
        let mut oauth = oauth_state(RefreshMethod::Proxy {
            proxy_url: "https://current-proxy.example.com".to_string(),
        });
        oauth.token_metadata = McpOAuthTokenMetadata {
            client_id: Some("current-proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some("https://current-proxy.example.com".to_string()),
        };
        let current_expiry = oauth.session.tokens.expires_at.expect("should expire");

        for expires_at in [
            Some(current_expiry - chrono::Duration::seconds(10)),
            Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
            Some(current_expiry),
            None,
        ] {
            let mut tokens = token_data("direct-external", current_expiry);
            tokens.expires_at = expires_at;
            let token_file = McpOAuthTokenFile {
                tokens,
                client_id: Some("direct-client".to_string()),
                client_secret: Some("direct-secret".to_string()),
                proxy_url: None,
            };

            let adopted = adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::RoutineRefreshRead,
            )
            .expect("should reject opposite-mode token file");

            assert!(!adopted);
            assert_eq!(oauth.session.access_token().secret(), "current");
            assert_eq!(oauth.session.tokens.expires_at, Some(current_expiry));
            let RefreshMethod::Proxy { proxy_url } = &oauth.refresh_method else {
                panic!("should remain proxy");
            };
            assert_eq!(proxy_url, "https://current-proxy.example.com");
            assert_eq!(
                oauth.token_metadata.client_id.as_deref(),
                Some("current-proxy-client")
            );
            assert!(oauth.token_metadata.client_secret.is_none());
            assert_eq!(
                oauth.token_metadata.proxy_url.as_deref(),
                Some("https://current-proxy.example.com")
            );
        }
    }

    #[test]
    fn routine_same_mode_adoption_rejects_stale_or_missing_expiry() {
        let mut oauth = oauth_state(RefreshMethod::Direct {
            oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
            client_id: "current-client".to_string(),
            client_secret: SecretString::from("current-secret".to_string()),
        });
        oauth.token_metadata = McpOAuthTokenMetadata {
            client_id: Some("current-client".to_string()),
            client_secret: Some("current-secret".to_string()),
            proxy_url: None,
        };
        let mut token_file = McpOAuthTokenFile {
            tokens: token_data("stale", chrono::Utc::now() - chrono::Duration::minutes(1)),
            client_id: Some("replacement-client".to_string()),
            client_secret: Some("replacement-secret".to_string()),
            proxy_url: None,
        };

        let adopted = adopt_external_token_file(
            &mut oauth,
            &token_file,
            ExternalTokenAdoptionPolicy::RoutineRefreshRead,
        )
        .expect("should inspect token file");

        assert!(!adopted);
        assert_eq!(oauth.session.access_token().secret(), "current");
        let RefreshMethod::Direct { client_id, .. } = &oauth.refresh_method else {
            panic!("should remain direct");
        };
        assert_eq!(client_id, "current-client");
        assert_eq!(
            oauth.token_metadata.client_secret.as_deref(),
            Some("current-secret")
        );

        token_file.tokens.expires_at = None;
        let adopted = adopt_external_token_file(
            &mut oauth,
            &token_file,
            ExternalTokenAdoptionPolicy::RoutineRefreshRead,
        )
        .expect("should inspect token file");
        assert!(!adopted);
        assert_eq!(oauth.session.access_token().secret(), "current");
    }

    #[test]
    fn routine_same_mode_read_keeps_active_refresh_metadata() {
        let mut oauth = oauth_state(RefreshMethod::Direct {
            oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
            client_id: "current-client".to_string(),
            client_secret: SecretString::from("current-secret".to_string()),
        });
        let token_file = McpOAuthTokenFile {
            tokens: token_data("fresher", chrono::Utc::now() + chrono::Duration::hours(1)),
            client_id: None,
            client_secret: None,
            proxy_url: None,
        };

        assert!(
            adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::RoutineRefreshRead,
            )
            .expect("fresher same-mode tokens should be adopted")
        );
        assert_eq!(
            oauth.token_metadata.client_id.as_deref(),
            Some("current-client")
        );
        assert_eq!(
            oauth.token_metadata.client_secret.as_deref(),
            Some("current-secret")
        );
    }

    #[test]
    fn watcher_same_mode_write_is_authoritative_for_every_expiry_shape() {
        let current_expiry = chrono::Utc::now() + chrono::Duration::minutes(10);
        for expires_at in [
            Some(current_expiry),
            Some(current_expiry - chrono::Duration::minutes(1)),
            Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
            None,
        ] {
            let mut oauth = oauth_state(RefreshMethod::Direct {
                oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
                client_id: "current-client".to_string(),
                client_secret: SecretString::from("current-secret".to_string()),
            });
            oauth.session.tokens.expires_at = Some(current_expiry);
            let mut tokens = token_data("completed-login", current_expiry);
            tokens.expires_at = expires_at;
            let token_file = McpOAuthTokenFile {
                tokens,
                client_id: Some("replacement-client".to_string()),
                client_secret: Some("replacement-secret".to_string()),
                proxy_url: None,
            };

            assert!(
                adopt_external_token_file(
                    &mut oauth,
                    &token_file,
                    ExternalTokenAdoptionPolicy::WatcherObservedWrite,
                )
                .expect("complete watcher write should be authoritative")
            );
            assert_eq!(oauth.session.access_token().secret(), "completed-login");
            assert_eq!(oauth.session.tokens.expires_at, expires_at);
            let RefreshMethod::Direct { client_id, .. } = oauth.refresh_method else {
                panic!("should remain direct");
            };
            assert_eq!(client_id, "replacement-client");
        }
    }

    #[test]
    fn invalid_watched_proxy_metadata_preserves_working_state() {
        let mut oauth = oauth_state(RefreshMethod::Direct {
            oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
            client_id: "current-client".to_string(),
            client_secret: SecretString::from("current-secret".to_string()),
        });
        let token_file = McpOAuthTokenFile {
            tokens: token_data(
                "untrusted-external",
                chrono::Utc::now() + chrono::Duration::hours(1),
            ),
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some("http://proxy.example.com".to_string()),
        };

        assert!(
            adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::WatcherObservedWrite,
            )
            .is_err()
        );
        assert_eq!(oauth.session.access_token().secret(), "current");
        assert!(matches!(oauth.refresh_method, RefreshMethod::Direct { .. }));
    }

    #[test]
    fn conflicting_metadata_preserves_watcher_and_routine_state() {
        for policy in [
            ExternalTokenAdoptionPolicy::WatcherObservedWrite,
            ExternalTokenAdoptionPolicy::RoutineRefreshRead,
        ] {
            let mut oauth = oauth_state(RefreshMethod::Direct {
                oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
                client_id: "current-client".to_string(),
                client_secret: SecretString::from("current-secret".to_string()),
            });
            oauth.token_metadata = McpOAuthTokenMetadata {
                client_id: Some("current-client".to_string()),
                client_secret: Some("current-secret".to_string()),
                proxy_url: None,
            };
            let current_expiry = oauth.session.tokens.expires_at;
            let token_file = McpOAuthTokenFile {
                tokens: token_data(
                    "untrusted-external",
                    chrono::Utc::now() + chrono::Duration::hours(1),
                ),
                client_id: Some("conflicting-client".to_string()),
                client_secret: Some("conflicting-secret".to_string()),
                proxy_url: Some("https://proxy.example.com".to_string()),
            };

            let error = adopt_external_token_file(&mut oauth, &token_file, policy)
                .expect_err("conflicting metadata must be rejected");

            assert!(
                error
                    .to_string()
                    .contains("both nonblank client_secret and proxy_url")
            );
            assert_eq!(oauth.session.access_token().secret(), "current");
            assert_eq!(oauth.session.refresh_token().secret(), "refresh-current");
            assert_eq!(oauth.session.tokens.expires_at, current_expiry);
            let RefreshMethod::Direct {
                client_id,
                client_secret,
                ..
            } = &oauth.refresh_method
            else {
                panic!("refresh mode must remain direct");
            };
            assert_eq!(client_id, "current-client");
            assert_eq!(client_secret.expose_secret(), "current-secret");
            assert_eq!(
                oauth.token_metadata,
                McpOAuthTokenMetadata {
                    client_id: Some("current-client".to_string()),
                    client_secret: Some("current-secret".to_string()),
                    proxy_url: None,
                }
            );
        }
    }

    #[tokio::test]
    async fn startup_rejects_conflicting_persisted_refresh_metadata() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        crate::oauth::save_token_file(&token_path, &token_file_with_all_metadata())
            .await
            .expect("should save conflicting token file");

        let error = build_oauth_ready_state(
            &AppConfig::default(),
            "https://cad.onshape.com/api/v6",
            Some(&token_path),
        )
        .err()
        .expect("startup must reject conflicting persisted metadata");

        assert!(
            error
                .to_string()
                .contains("both nonblank client_secret and proxy_url")
        );
    }

    #[test]
    fn configured_and_persisted_proxy_urls_are_validated() {
        let mut config = AppConfig::default();
        config.auth.proxy_url = Some("http://proxy.example.com".to_string());
        assert!(determine_refresh_method(&config, None).is_err());
        assert!(determine_pending_refresh_method(&config).is_err());
        config.auth.proxy_url = Some("  ".to_string());
        assert!(determine_refresh_method(&config, None).is_err());
        assert!(determine_pending_refresh_method(&config).is_err());

        config.auth.proxy_url = None;
        let mut token_file = token_file_with_all_metadata();
        token_file.proxy_url = Some("http://proxy.example.com".to_string());
        assert!(determine_refresh_method(&config, Some(&token_file)).is_err());
        assert!(refresh_method_from_token_file(&token_file).is_err());

        for accepted in ["https://proxy.example.com/base/", "http://127.0.0.1:8787"] {
            config.auth.proxy_url = Some(accepted.to_string());
            assert!(matches!(
                determine_refresh_method(&config, None),
                Ok(Some(RefreshMethod::Proxy { .. }))
            ));
        }
    }

    #[test]
    fn startup_rejects_invalid_explicit_proxy_even_when_basic_is_selected() {
        let mut config = AppConfig::default();
        config.auth.method = AuthMethod::Basic;
        config.auth.access_key = Some(SecretString::from("access"));
        config.auth.secret_key = Some(SecretString::from("secret"));
        config.auth.proxy_url = Some("http://proxy.example.com".to_string());

        let error = build_api_state(&config, "https://cad.onshape.com/api/v6")
            .err()
            .expect("startup should reject invalid explicit proxy URL");
        assert!(
            error
                .to_string()
                .contains("invalid configured auth.proxy_url")
        );
    }

    #[tokio::test]
    async fn snapshot_detects_same_content_atomic_republication() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let path = dir.path().join("tokens.json");
        let token_file = token_file_with_all_metadata();
        crate::oauth::save_token_file(&path, &token_file)
            .await
            .expect("initial publication should succeed");
        let before = TokenFileSnapshot::capture(&path);

        crate::oauth::save_token_file(&path, &token_file)
            .await
            .expect("identical republication should succeed");
        let after = TokenFileSnapshot::capture(&path);

        assert_ne!(before, after);
    }

    #[test]
    fn watcher_rejects_empty_token_material_without_mutation() {
        let mut oauth = oauth_state(RefreshMethod::Direct {
            oauth_client: Box::new(onshape_oauth_client("current-client", "current-secret")),
            client_id: "current-client".to_string(),
            client_secret: SecretString::from("current-secret".to_string()),
        });
        let mut token_file = token_file_with_all_metadata();
        token_file.proxy_url = None;
        token_file.tokens.refresh_token = RefreshToken::new(String::new());

        assert!(
            !adopt_external_token_file(
                &mut oauth,
                &token_file,
                ExternalTokenAdoptionPolicy::WatcherObservedWrite,
            )
            .expect("empty token material should be ignored")
        );
        assert_eq!(oauth.session.access_token().secret(), "current");
    }

    #[tokio::test]
    async fn proxy_refresh_omitted_or_blank_rotation_keeps_existing_refresh_token() {
        for refresh_token in [None, Some(""), Some(" \t ")] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("should bind test proxy");
            let address = listener.local_addr().expect("should have local address");
            let mut response = serde_json::json!({
                "access_token": "refreshed-access",
                "expires_in": 3600,
                "token_type": "bearer"
            });
            if let Some(refresh_token) = refresh_token {
                response["refresh_token"] = serde_json::Value::String(refresh_token.to_string());
            }
            let app = axum::Router::new().route(
                "/token/refresh",
                axum::routing::post(move || {
                    let response = response.clone();
                    async move { axum::Json(response) }
                }),
            );
            let server = tokio::spawn(async move {
                axum::serve(listener, app)
                    .await
                    .expect("test proxy should serve");
            });
            let proxy_url = format!("http://{address}");
            let mut oauth = oauth_state(RefreshMethod::Proxy {
                proxy_url: proxy_url.clone(),
            });
            let original_refresh = oauth.session.refresh_token().secret().clone();

            try_refresh_proxy(&mut oauth, &proxy_url)
                .await
                .expect("proxy refresh should succeed without rotation");

            assert_eq!(oauth.session.access_token().secret(), "refreshed-access");
            assert_eq!(oauth.session.refresh_token().secret(), &original_refresh);
            server.abort();
        }
    }

    async fn rotating_proxy() -> (
        String,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let exchanges = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind test proxy");
        let address = listener.local_addr().expect("should have local address");
        let app = axum::Router::new().route(
            "/token/refresh",
            axum::routing::post({
                let exchanges = std::sync::Arc::clone(&exchanges);
                move || {
                    let exchanges = std::sync::Arc::clone(&exchanges);
                    async move {
                        exchanges.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        axum::Json(serde_json::json!({
                            "access_token": "proxy-refreshed",
                            "refresh_token": "rotated-refresh",
                            "expires_in": 3600,
                            "token_type": "bearer"
                        }))
                    }
                }
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test proxy should serve");
        });
        (format!("http://{address}"), exchanges, server)
    }

    fn configure_proxy_state(
        oauth: &mut OAuthApiState,
        token_path: &std::path::Path,
        proxy_url: &str,
        snapshot: TokenFileSnapshot,
    ) {
        oauth.token_path = token_path.to_path_buf();
        oauth.token_metadata = McpOAuthTokenMetadata {
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some(proxy_url.to_string()),
        };
        oauth.last_observed_token_snapshot = snapshot;
    }

    #[tokio::test]
    async fn concurrent_rotating_refreshes_exchange_once_and_converge() {
        let (proxy_url, exchanges, server) = rotating_proxy().await;
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        let mut first = oauth_state(RefreshMethod::Proxy {
            proxy_url: proxy_url.clone(),
        });
        let initial_file = first
            .token_metadata
            .with_tokens(first.session.tokens.clone());
        let initial_file = McpOAuthTokenMetadata {
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some(proxy_url.clone()),
        }
        .with_tokens(initial_file.tokens);
        crate::oauth::save_token_file(&token_path, &initial_file)
            .await
            .expect("should save initial proxy tokens");
        let snapshot = TokenFileSnapshot::capture(&token_path);
        configure_proxy_state(&mut first, &token_path, &proxy_url, snapshot.clone());
        let mut second = oauth_state(RefreshMethod::Proxy {
            proxy_url: proxy_url.clone(),
        });
        configure_proxy_state(&mut second, &token_path, &proxy_url, snapshot);

        let (first_result, second_result) =
            tokio::join!(try_refresh(&mut first), try_refresh(&mut second));
        first_result.expect("first refresh should succeed");
        second_result.expect("second refresh should adopt the first publication");

        assert_eq!(exchanges.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(first.session.access_token().secret(), "proxy-refreshed");
        assert_eq!(second.session.access_token().secret(), "proxy-refreshed");
        assert_eq!(first.session.refresh_token().secret(), "rotated-refresh");
        assert_eq!(second.session.refresh_token().secret(), "rotated-refresh");
        server.abort();
    }

    #[tokio::test]
    async fn opposite_mode_login_before_refresh_lock_is_adopted_without_exchange() {
        let (proxy_url, exchanges, server) = rotating_proxy().await;
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        let mut oauth = oauth_state(RefreshMethod::Proxy {
            proxy_url: proxy_url.clone(),
        });
        let baseline = McpOAuthTokenMetadata {
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some(proxy_url.clone()),
        }
        .with_tokens(oauth.session.tokens.clone());
        crate::oauth::save_token_file(&token_path, &baseline)
            .await
            .expect("should save baseline");
        configure_proxy_state(
            &mut oauth,
            &token_path,
            &proxy_url,
            TokenFileSnapshot::capture(&token_path),
        );

        let direct_login = McpOAuthTokenFile {
            tokens: token_data(
                "direct-login",
                chrono::Utc::now() + chrono::Duration::minutes(30),
            ),
            client_id: Some("direct-client".to_string()),
            client_secret: Some("direct-secret".to_string()),
            proxy_url: None,
        };
        crate::oauth::save_token_file(&token_path, &direct_login)
            .await
            .expect("login should publish before refresh acquires the lock");

        try_refresh(&mut oauth)
            .await
            .expect("refresh should adopt the completed login");

        assert_eq!(exchanges.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(oauth.session.access_token().secret(), "direct-login");
        assert!(matches!(oauth.refresh_method, RefreshMethod::Direct { .. }));
        server.abort();
    }

    #[tokio::test]
    async fn held_lock_prevents_refresh_exchange_until_released() {
        let (proxy_url, exchanges, server) = rotating_proxy().await;
        let dir = tempfile::tempdir().expect("should create temp dir");
        let token_path = dir.path().join("tokens.json");
        let mut oauth = oauth_state(RefreshMethod::Proxy {
            proxy_url: proxy_url.clone(),
        });
        let baseline = McpOAuthTokenMetadata {
            client_id: Some("proxy-client".to_string()),
            client_secret: None,
            proxy_url: Some(proxy_url.clone()),
        }
        .with_tokens(oauth.session.tokens.clone());
        crate::oauth::save_token_file(&token_path, &baseline)
            .await
            .expect("should save baseline");
        configure_proxy_state(
            &mut oauth,
            &token_path,
            &proxy_url,
            TokenFileSnapshot::capture(&token_path),
        );
        let held_lock = crate::oauth::TokenFileLock::acquire(&token_path)
            .await
            .expect("should hold transaction lock");

        let refresh = try_refresh(&mut oauth);
        tokio::pin!(refresh);
        assert!(
            tokio::time::timeout(Duration::from_millis(75), &mut refresh)
                .await
                .is_err()
        );
        assert_eq!(exchanges.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(held_lock);
        refresh.await.expect("refresh should proceed after release");
        assert_eq!(exchanges.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn changed_invalid_or_missing_file_fails_closed_before_exchange() {
        for change in ["invalid", "missing"] {
            let (proxy_url, exchanges, server) = rotating_proxy().await;
            let dir = tempfile::tempdir().expect("should create temp dir");
            let token_path = dir.path().join("tokens.json");
            let mut oauth = oauth_state(RefreshMethod::Proxy {
                proxy_url: proxy_url.clone(),
            });
            let baseline = McpOAuthTokenMetadata {
                client_id: Some("proxy-client".to_string()),
                client_secret: None,
                proxy_url: Some(proxy_url.clone()),
            }
            .with_tokens(oauth.session.tokens.clone());
            crate::oauth::save_token_file(&token_path, &baseline)
                .await
                .expect("should save baseline");
            configure_proxy_state(
                &mut oauth,
                &token_path,
                &proxy_url,
                TokenFileSnapshot::capture(&token_path),
            );
            if change == "invalid" {
                std::fs::write(&token_path, b"not json").expect("should corrupt token file");
            } else {
                std::fs::remove_file(&token_path).expect("should remove token file");
            }

            let error = try_refresh(&mut oauth)
                .await
                .expect_err("changed invalid state must fail closed");
            assert!(matches!(error, RefreshError::ExternalTokenChange(_)));
            assert_eq!(exchanges.load(std::sync::atomic::Ordering::SeqCst), 0);
            server.abort();
        }
    }

    // --- build_ipv4_retry_client tests ---

    #[test]
    fn ipv4_retry_returns_client_for_ipv6_source() {
        let body = r#"{"error":"forbidden","source_ip":"2601:980:c200:8530:bfc8:c956:e7c1:1d07"}"#;
        assert!(build_ipv4_retry_client(body).is_some());
    }

    #[test]
    fn ipv4_retry_returns_none_for_ipv4_source() {
        let body = r#"{"error":"forbidden","source_ip":"71.58.134.128"}"#;
        assert!(build_ipv4_retry_client(body).is_none());
    }

    #[test]
    fn ipv4_retry_returns_none_for_unparsable_body() {
        assert!(build_ipv4_retry_client("not json").is_none());
    }

    #[test]
    fn ipv4_retry_returns_none_for_missing_source_ip() {
        let body = r#"{"error":"forbidden"}"#;
        assert!(build_ipv4_retry_client(body).is_none());
    }

    #[test]
    fn ipv4_retry_returns_client_for_ipv6_loopback() {
        let body = r#"{"error":"forbidden","source_ip":"::1"}"#;
        assert!(build_ipv4_retry_client(body).is_some());
    }

    // --- dispatch_tool_effect file-write gating tests ---

    /// Helper: build a `ToolEffect::WriteFiles` targeting `path` with dummy data.
    fn dummy_write_files(path: std::path::PathBuf) -> tools::ToolEffect {
        tools::ToolEffect::WriteFiles {
            files: vec![tools::FileWrite {
                path,
                data: b"png-bytes".to_vec(),
            }],
            continuation: tools::Continuation::FormatScreenshotWrite {
                label: "test".to_string(),
                view_matrix: "front".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn file_writes_blocked_when_disallowed() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file_path = dir.path().join("should_not_exist.png");

        let validation = tokio::sync::Mutex::new(ValidationState::default());
        let mut state = ApiState::NotConfigured {
            configured_method: AuthMethod::OAuth,
            detail: "test".to_string(),
        };

        let result = dispatch_tool_effect(
            dummy_write_files(file_path.clone()),
            &mut state,
            &validation,
            None,
            false, // disallow file writes
            false, // disallow file reads
        )
        .await
        .expect("should not return protocol error");

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0]
            .raw
            .as_text()
            .expect("should be text content");
        assert!(
            text.text.contains("not supported over the HTTP transport"),
            "unexpected error message: {}",
            text.text
        );
        assert!(
            !file_path.exists(),
            "file should not have been written to disk"
        );
    }

    #[tokio::test]
    async fn file_writes_allowed_when_permitted() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file_path = dir.path().join("output.png");

        let validation = tokio::sync::Mutex::new(ValidationState::default());
        let mut state = ApiState::NotConfigured {
            configured_method: AuthMethod::OAuth,
            detail: "test".to_string(),
        };

        let result = dispatch_tool_effect(
            dummy_write_files(file_path.clone()),
            &mut state,
            &validation,
            None,
            true, // allow file writes
            true, // allow file reads
        )
        .await
        .expect("should not return protocol error");

        assert_eq!(result.is_error, Some(false));
        assert!(file_path.exists(), "file should have been written to disk");
        assert_eq!(
            std::fs::read(&file_path).expect("should read file"),
            b"png-bytes"
        );
    }
}
