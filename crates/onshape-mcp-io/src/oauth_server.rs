//! OAuth 2.0 Authorization Server for the HTTP transport.
//!
//! Implements the server-side OAuth flow that allows Claude.ai (or any MCP
//! client) to authenticate users via their Onshape accounts. The flow:
//!
//! 1. Client discovers OAuth metadata via well-known endpoints
//! 2. Client registers dynamically (DCR)
//! 3. Client redirects user to `/oauth/authorize`
//! 4. Server redirects to Onshape OAuth, user approves
//! 5. Onshape redirects back to `/oauth/callback`
//! 6. Server verifies user is on the allowlist
//! 7. Server issues MCP access token to the client
//! 8. Client uses bearer token on `/mcp` requests
//!
//! All session state is in-memory. Server restarts invalidate all sessions.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect};
use axum::{Json, Router, middleware, routing};
use oauth2::{
    AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, Scope, TokenResponse,
};
use rand::RngExt;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use onshape_client_core::oauth::onshape_oauth_client;

// ============================================================================
// Types
// ============================================================================

/// Onshape tokens stored for an authenticated user.
///
/// Secret fields are private to enforce controlled access via
/// [`expose_secret()`](secrecy::ExposeSecret::expose_secret) at call sites.
#[derive(Clone, Debug)]
pub(crate) struct UserOnshapeTokens {
    access_token: SecretString,
    refresh_token: SecretString,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl UserOnshapeTokens {
    /// Create a new set of user Onshape tokens.
    pub(crate) const fn new(
        access_token: SecretString,
        refresh_token: SecretString,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at,
        }
    }

    /// Borrow the access token.
    pub(crate) const fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    /// Borrow the refresh token.
    pub(crate) const fn refresh_token(&self) -> &SecretString {
        &self.refresh_token
    }

    /// When this token expires, if known.
    pub(crate) const fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }
}

/// Context inserted into HTTP request extensions by the auth middleware.
///
/// Accessible in `call_tool()` via `request::Parts::extensions`.
#[derive(Clone, Debug)]
pub(crate) struct UserContext {
    /// Onshape user ID (used for per-user token management and logging).
    pub user_id: String,
    /// The user's Onshape tokens for API calls.
    pub onshape_tokens: UserOnshapeTokens,
}

/// Pending authorization state — stored between `/oauth/authorize` and
/// `/oauth/callback`.
#[derive(Debug)]
struct PendingAuth {
    /// Client ID of the MCP client (e.g. Claude.ai's dynamically registered client).
    client_id: String,
    /// Redirect URI for the MCP client.
    redirect_uri: String,
    /// PKCE code challenge from the MCP client's auth request (RFC 7636).
    pkce_code_challenge: Option<String>,
    /// The CSRF state token from the MCP client's auth request.
    /// `None` when the client omitted the optional `state` parameter.
    mcp_state: Option<String>,
    /// PKCE verifier for the Onshape leg of the flow.
    onshape_pkce_verifier: PkceCodeVerifier,
}

/// A dynamically registered MCP client.
#[derive(Debug, Clone)]
struct RegisteredClient {
    #[allow(dead_code)]
    client_id: String,
    client_secret: String,
    redirect_uris: Vec<String>,
}

/// An issued MCP authorization code.
#[derive(Debug)]
struct IssuedAuthCode {
    /// The MCP client this code was issued for.
    client_id: String,
    /// The redirect URI used in the authorization request.
    redirect_uri: String,
    /// PKCE code challenge from the client's authorization request (RFC 7636).
    pkce_code_challenge: Option<String>,
    /// Onshape user ID associated with this code.
    user_id: String,
    /// When this code was issued (used to enforce [`AUTH_CODE_TTL_SECS`]).
    created_at: chrono::DateTime<chrono::Utc>,
}

/// An issued MCP access token → user mapping.
#[derive(Debug, Clone)]
struct IssuedToken {
    /// Onshape user ID.
    user_id: String,
    /// The client that this token was issued for.
    client_id: String,
    /// When the token was issued.
    #[allow(dead_code)]
    issued_at: chrono::DateTime<chrono::Utc>,
    /// When the token expires.
    expires_at: chrono::DateTime<chrono::Utc>,
}

/// Shared state for the OAuth server.
pub(crate) struct OAuthServerState {
    /// Dynamically registered clients.
    clients: RwLock<HashMap<String, RegisteredClient>>,
    /// Pending authorization flows (keyed by Onshape CSRF state).
    pending_auth: RwLock<HashMap<String, PendingAuth>>,
    /// Issued authorization codes (keyed by code value).
    auth_codes: RwLock<HashMap<String, IssuedAuthCode>>,
    /// Issued access tokens → user mapping.
    tokens: RwLock<HashMap<String, IssuedToken>>,
    /// Issued refresh tokens → user mapping (separate from access tokens).
    refresh_tokens: RwLock<HashMap<String, IssuedToken>>,
    /// User Onshape tokens (keyed by Onshape user ID).
    pub(crate) user_tokens: RwLock<HashMap<String, UserOnshapeTokens>>,
    /// Per-user locks for serializing Onshape token refresh operations.
    ///
    /// Prevents concurrent refreshes for the same user from consuming the
    /// same refresh token twice (Onshape may invalidate the old refresh token
    /// when a new one is issued).
    refresh_locks: RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Allowlist of Onshape user IDs.
    allowed_users: HashSet<String>,
    /// Onshape OAuth app client ID (operator's app).
    onshape_client_id: String,
    /// Onshape OAuth app client secret (operator's app).
    onshape_client_secret: SecretString,
    /// Public URL of this MCP server (validated at construction time).
    public_url: url::Url,
}

/// Errors that can occur during per-user Onshape token refresh.
#[derive(Debug, thiserror::Error)]
pub(crate) enum UserTokenRefreshError {
    /// User not found in the token store.
    #[error("user not found in token store")]
    UserNotFound,
    /// Transient exchange failure (network, server error).
    #[error("token refresh request failed: {0}")]
    Exchange(String),
    /// Permanent exchange failure (refresh token revoked/expired).
    #[error("token refresh permanently failed: {0}")]
    PermanentExchange(String),
    /// Failed to build HTTP client for the refresh request.
    #[error("failed to build HTTP client: {0}")]
    HttpClient(String),
}

/// MCP access token lifetime (1 hour, matching Onshape).
const TOKEN_LIFETIME_SECS: i64 = 3600;

/// Maximum lifetime of an authorization code (RFC 6749 §4.1.2 recommends ≤10 min).
const AUTH_CODE_TTL_SECS: i64 = 600;

/// Remove any existing tokens for the given user+client pair, then insert the new one.
///
/// This ensures at most one access token (or refresh token) exists per
/// `(user_id, client_id)` pair, revoking stale tokens from prior grants.
fn replace_token(
    tokens: &mut HashMap<String, IssuedToken>,
    new_key: String,
    new_value: IssuedToken,
) {
    let user_id = &new_value.user_id;
    let client_id = &new_value.client_id;
    tokens.retain(|_, t| !(t.user_id == *user_id && t.client_id == *client_id));
    tokens.insert(new_key, new_value);
}

// ============================================================================
// State Construction
// ============================================================================

impl OAuthServerState {
    /// Create a new OAuth server state.
    ///
    /// `public_url` must be a validated URL with no query or fragment.
    /// Trailing path slashes are stripped to ensure consistent path extension.
    pub(crate) fn new(
        public_url: url::Url,
        onshape_client_id: String,
        onshape_client_secret: SecretString,
        allowed_user_ids: Vec<String>,
    ) -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            pending_auth: RwLock::new(HashMap::new()),
            auth_codes: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(HashMap::new()),
            user_tokens: RwLock::new(HashMap::new()),
            refresh_locks: RwLock::new(HashMap::new()),
            allowed_users: allowed_user_ids.into_iter().collect(),
            onshape_client_id,
            onshape_client_secret,
            public_url,
        }
    }

    /// Build a URL by extending the public URL's path with additional segments.
    ///
    /// # Panics
    ///
    /// Cannot panic: `public_url` is validated at construction time to use
    /// an `http`/`https` scheme with a host, so `path_segments_mut()` always
    /// succeeds (it only fails for cannot-be-a-base URLs like `data:` or
    /// `mailto:`).
    #[allow(clippy::expect_used)]
    fn url_with_path(&self, segments: &[&str]) -> String {
        let mut url = self.public_url.clone();
        url.path_segments_mut()
            .expect("public_url is validated to have a host, so path_segments_mut cannot fail")
            .extend(segments);
        url.into()
    }

    /// Validate a bearer token and return the user context if valid.
    pub(crate) async fn validate_token(&self, token: &str) -> Option<UserContext> {
        let (user_id, expires_at) = self
            .tokens
            .read()
            .await
            .get(token)
            .map(|issued| (issued.user_id.clone(), issued.expires_at))?;
        if chrono::Utc::now() > expires_at {
            return None;
        }
        let onshape_tokens = self.user_tokens.read().await.get(&user_id)?.clone();
        Some(UserContext {
            user_id,
            onshape_tokens,
        })
    }

    /// Refresh a user's Onshape tokens using the server's client credentials.
    ///
    /// Acquires a per-user lock to prevent concurrent refreshes from consuming
    /// the same refresh token twice (Onshape may invalidate old refresh tokens
    /// when new ones are issued).
    ///
    /// If `stale_before` is provided and the stored token already expires after
    /// that timestamp, the refresh is skipped — another request already refreshed
    /// while we waited for the lock.
    pub(crate) async fn refresh_user_onshape_tokens(
        &self,
        user_id: &str,
        stale_before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<UserOnshapeTokens, UserTokenRefreshError> {
        // Acquire the per-user refresh lock.
        let lock = self.get_user_refresh_lock(user_id).await;
        let _guard = lock.lock().await;

        // Re-read tokens — they may have been refreshed while we waited.
        let current_tokens = self
            .user_tokens
            .read()
            .await
            .get(user_id)
            .cloned()
            .ok_or(UserTokenRefreshError::UserNotFound)?;

        // Double-check: skip if another request already refreshed.
        if let Some(stale) = stale_before
            && let Some(current_expires) = current_tokens.expires_at()
            && current_expires > stale
        {
            return Ok(current_tokens);
        }

        eprintln!("[oauth] refreshing Onshape tokens for user {user_id}");

        // Build OAuth client using server's Onshape app credentials.
        // Use RequestBody auth (client_secret_post) — Onshape requires
        // credentials in the POST body, not HTTP Basic auth.
        let onshape_client = onshape_oauth_client(
            &self.onshape_client_id,
            self.onshape_client_secret.expose_secret(),
        )
        .set_auth_type(oauth2::AuthType::RequestBody);

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| UserTokenRefreshError::HttpClient(e.to_string()))?;

        let refresh_token =
            oauth2::RefreshToken::new(current_tokens.refresh_token().expose_secret().to_string());

        let response = onshape_client
            .exchange_refresh_token(&refresh_token)
            .request_async(&oauth2_reqwest::ReqwestClient::from(http_client))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if crate::is_permanent_refresh_failure(&msg) {
                    UserTokenRefreshError::PermanentExchange(msg)
                } else {
                    UserTokenRefreshError::Exchange(msg)
                }
            })?;

        // Build new tokens from the response.
        let now = chrono::Utc::now();
        let access_token = response.access_token().secret().clone();

        // Per RFC 6749 §6: if the server omits refresh_token in the
        // response, keep the existing one.
        let new_refresh_token = response.refresh_token().map_or_else(
            || current_tokens.refresh_token().expose_secret().to_string(),
            |t| t.secret().clone(),
        );

        let expires_at = response
            .expires_in()
            .and_then(|d| chrono::Duration::from_std(d).ok())
            .map(|d| now + d);

        let new_tokens = UserOnshapeTokens::new(
            SecretString::from(access_token),
            SecretString::from(new_refresh_token),
            expires_at,
        );

        // Update stored tokens.
        self.user_tokens
            .write()
            .await
            .insert(user_id.to_string(), new_tokens.clone());

        eprintln!("[oauth] Onshape token refresh succeeded for user {user_id}");

        Ok(new_tokens)
    }

    /// Get or create the per-user refresh lock.
    async fn get_user_refresh_lock(&self, user_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        // Fast path: lock already exists.
        if let Some(lock) = self.refresh_locks.read().await.get(user_id) {
            return Arc::clone(lock);
        }

        // Slow path: create a new lock.
        let mut locks = self.refresh_locks.write().await;
        // Re-check after acquiring write lock (another task may have created it).
        locks
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

// ============================================================================
// Helper: Random Token Generation
// ============================================================================

/// Generate a cryptographically random hex string.
///
/// Uses `ThreadRng` (`ChaCha12` seeded from OS entropy), which is a CSPRNG
/// suitable for security-critical material per the `rand` crate docs.
/// `OsRng` would be preferable for direct OS entropy, but its fallible
/// API (`try_fill_bytes`) would require error propagation through all
/// callers. `ThreadRng` is an acceptable alternative: it is automatically
/// seeded from `OsRng` and periodically reseeded.
fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill(&mut buf[..]);
    hex_encode(&buf)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // write! to a String is infallible, but we use `let _ =` to satisfy
        // the `unwrap_used` lint without introducing a panic path.
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ============================================================================
// Metadata Endpoints
// ============================================================================

/// Health check endpoint.
///
/// `GET /health` — returns 200 OK with a simple JSON body.
/// Used by load balancers and container orchestrators (e.g. Fly.io).
async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

/// RFC 9728: Protected Resource Metadata.
///
/// `GET /.well-known/oauth-protected-resource`
async fn protected_resource_metadata(
    State(state): State<Arc<OAuthServerState>>,
) -> impl IntoResponse {
    Json(serde_json::json!({
        "resource": state.url_with_path(&["mcp"]),
        "authorization_servers": [state.public_url.as_str()],
        "bearer_methods_supported": ["header"],
    }))
}

/// RFC 8414: Authorization Server Metadata.
///
/// `GET /.well-known/oauth-authorization-server`
async fn authorization_server_metadata(
    State(state): State<Arc<OAuthServerState>>,
) -> impl IntoResponse {
    Json(serde_json::json!({
        "issuer": state.public_url.as_str(),
        "authorization_endpoint": state.url_with_path(&["oauth", "authorize"]),
        "token_endpoint": state.url_with_path(&["oauth", "token"]),
        "registration_endpoint": state.url_with_path(&["oauth", "register"]),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": [],
    }))
}

// ============================================================================
// Dynamic Client Registration
// ============================================================================

/// Request body for `POST /oauth/register`.
#[derive(Deserialize)]
struct RegisterRequest {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(default)]
    grant_types: Vec<String>,
    #[serde(default)]
    response_types: Vec<String>,
    #[allow(dead_code)]
    token_endpoint_auth_method: Option<String>,
}

/// Response for `POST /oauth/register`.
#[derive(Debug, Serialize)]
struct RegisterResponse {
    client_id: String,
    client_secret: String,
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
}

/// Supported grant types for this server.
const SUPPORTED_GRANT_TYPES: &[&str] = &["authorization_code", "refresh_token"];
/// Supported response types for this server.
const SUPPORTED_RESPONSE_TYPES: &[&str] = &["code"];

/// Validate that each redirect URI is syntactically valid and uses an allowed
/// scheme per the MCP spec (Security Considerations §5):
///
///   "Redirect URIs MUST be either localhost URLs or HTTPS URLs"
///
/// Accepts `https://` (any host) and `http://` only for loopback hosts
/// (`localhost`, any `127.0.0.0/8` address, or `[::1]`).
fn validate_redirect_uris(
    uris: &[String],
) -> Result<(), (http::StatusCode, Json<serde_json::Value>)> {
    for uri in uris {
        let Ok(parsed) = url::Url::parse(uri) else {
            return Err((
                http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_client_metadata",
                    "error_description": format!("invalid redirect_uri: {uri}"),
                })),
            ));
        };

        let scheme_ok = match parsed.scheme() {
            "https" => true,
            "http" => match parsed.host() {
                Some(url::Host::Domain("localhost")) => true,
                Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                _ => false,
            },
            _ => false,
        };

        if !scheme_ok {
            return Err((
                http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_client_metadata",
                    "error_description": format!(
                        "redirect_uri must use https:// or http:// with a loopback host: {uri}"
                    ),
                })),
            ));
        }
    }

    Ok(())
}

async fn register_client(
    State(state): State<Arc<OAuthServerState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (http::StatusCode, Json<serde_json::Value>)> {
    eprintln!(
        "[oauth] DCR: registering client name={:?} redirect_uris={:?}",
        req.client_name, req.redirect_uris
    );

    // Validate redirect_uris is non-empty.
    if req.redirect_uris.is_empty() {
        return Err((
            http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client_metadata",
                "error_description": "redirect_uris must not be empty",
            })),
        ));
    }

    // Validate redirect_uri syntax and scheme (MCP spec compliance).
    validate_redirect_uris(&req.redirect_uris)?;

    // Validate grant_types (default if empty, reject unsupported).
    let grant_types = if req.grant_types.is_empty() {
        vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ]
    } else {
        for gt in &req.grant_types {
            if !SUPPORTED_GRANT_TYPES.contains(&gt.as_str()) {
                return Err((
                    http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "invalid_client_metadata",
                        "error_description": format!("unsupported grant_type: {gt}"),
                    })),
                ));
            }
        }
        req.grant_types
    };

    // Validate response_types (default if empty, reject unsupported).
    let response_types = if req.response_types.is_empty() {
        vec!["code".to_string()]
    } else {
        for rt in &req.response_types {
            if !SUPPORTED_RESPONSE_TYPES.contains(&rt.as_str()) {
                return Err((
                    http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "invalid_client_metadata",
                        "error_description": format!("unsupported response_type: {rt}"),
                    })),
                ));
            }
        }
        req.response_types
    };

    let client_id = random_hex(16);
    let client_secret = random_hex(32);

    let registered = RegisteredClient {
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        redirect_uris: req.redirect_uris.clone(),
    };

    state
        .clients
        .write()
        .await
        .insert(client_id.clone(), registered);

    eprintln!("[oauth] DCR: issued client_id={client_id}");

    Ok(Json(RegisterResponse {
        client_id,
        client_secret,
        client_name: req.client_name,
        redirect_uris: req.redirect_uris,
        grant_types,
        response_types,
        token_endpoint_auth_method: "client_secret_post".to_string(),
    }))
}

// ============================================================================
// Authorization Endpoint
// ============================================================================

/// Query parameters for `GET /oauth/authorize`.
#[derive(Deserialize)]
struct AuthorizeParams {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    #[allow(dead_code)]
    scope: Option<String>,
}

async fn authorize(
    State(state): State<Arc<OAuthServerState>>,
    Query(params): Query<AuthorizeParams>,
) -> Result<Redirect, (http::StatusCode, String)> {
    eprintln!(
        "[oauth] authorize: client_id={} redirect_uri={}",
        params.client_id, params.redirect_uri
    );

    // Validate response_type.
    if params.response_type != "code" {
        eprintln!(
            "[oauth] authorize: rejected unsupported response_type={}",
            params.response_type
        );
        return Err((
            http::StatusCode::BAD_REQUEST,
            "unsupported response_type".to_string(),
        ));
    }

    // Validate client_id.
    let clients = state.clients.read().await;
    let Some(client) = clients.get(&params.client_id) else {
        eprintln!("[oauth] authorize: rejected unknown client_id");
        return Err((
            http::StatusCode::BAD_REQUEST,
            "unknown client_id".to_string(),
        ));
    };

    // Validate redirect_uri.
    if !client.redirect_uris.contains(&params.redirect_uri) {
        eprintln!("[oauth] authorize: rejected unregistered redirect_uri");
        return Err((
            http::StatusCode::BAD_REQUEST,
            "redirect_uri not registered".to_string(),
        ));
    }
    drop(clients);

    // Validate PKCE code_challenge_method (we only support S256, per metadata).
    if params.code_challenge.is_some() && params.code_challenge_method.as_deref() != Some("S256") {
        eprintln!(
            "[oauth] authorize: rejected unsupported code_challenge_method={:?}",
            params.code_challenge_method
        );
        return Err((
            http::StatusCode::BAD_REQUEST,
            "code_challenge_method must be S256".to_string(),
        ));
    }

    // Store the MCP client's PKCE code challenge (if provided) for later validation.
    let pkce_code_challenge = params.code_challenge.clone();

    // Generate Onshape OAuth parameters.
    let (onshape_pkce_challenge, onshape_pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let onshape_csrf = CsrfToken::new_random();

    // Store pending auth state keyed by the Onshape CSRF token.
    let pending = PendingAuth {
        client_id: params.client_id.clone(),
        redirect_uri: params.redirect_uri.clone(),
        pkce_code_challenge,
        mcp_state: params.state.clone(),
        onshape_pkce_verifier,
    };
    state
        .pending_auth
        .write()
        .await
        .insert(onshape_csrf.secret().clone(), pending);

    // Build the Onshape authorization URL.
    let onshape_client = onshape_oauth_client(
        &state.onshape_client_id,
        state.onshape_client_secret.expose_secret(),
    );
    let callback_url = state.url_with_path(&["oauth", "callback"]);
    let redirect_url = oauth2::RedirectUrl::new(callback_url).map_err(|e| {
        (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid callback URL: {e}"),
        )
    })?;

    let (auth_url, _) = onshape_client
        .set_redirect_uri(redirect_url)
        .authorize_url(|| onshape_csrf)
        .set_pkce_challenge(onshape_pkce_challenge)
        .add_scope(Scope::new("OAuth2Read".to_string()))
        .add_scope(Scope::new("OAuth2Write".to_string()))
        .url();

    Ok(Redirect::to(auth_url.as_str()))
}

// ============================================================================
// Onshape Callback
// ============================================================================

/// Query parameters for `GET /oauth/callback`.
#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Onshape session info response.
#[derive(Deserialize)]
struct SessionInfo {
    id: String,
    #[allow(dead_code)]
    name: Option<String>,
}

/// Exchange an Onshape authorization code for tokens.
async fn exchange_onshape_code(
    state: &OAuthServerState,
    onshape_code: String,
    pkce_verifier: PkceCodeVerifier,
) -> Result<
    (
        oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>,
        reqwest::Client,
    ),
    (http::StatusCode, String),
> {
    eprintln!("[oauth] callback: exchanging Onshape authorization code for tokens");

    // Use RequestBody (client_secret_post) auth — Onshape requires credentials
    // in the POST body rather than an HTTP Basic Authorization header.
    let onshape_client = onshape_oauth_client(
        &state.onshape_client_id,
        state.onshape_client_secret.expose_secret(),
    )
    .set_auth_type(oauth2::AuthType::RequestBody);

    let callback_url = state.url_with_path(&["oauth", "callback"]);
    let redirect_url = oauth2::RedirectUrl::new(callback_url).map_err(|e| {
        (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid callback URL: {e}"),
        )
    })?;

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build HTTP client: {e}"),
            )
        })?;

    let token_response = onshape_client
        .set_redirect_uri(redirect_url)
        .exchange_code(AuthorizationCode::new(onshape_code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&oauth2_reqwest::ReqwestClient::from(http_client.clone()))
        .await
        .map_err(|e| {
            eprintln!("[oauth] callback: token exchange failed: {e}");
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("token exchange failed: {e}"),
            )
        })?;

    eprintln!("[oauth] callback: Onshape token exchange succeeded");
    Ok((token_response, http_client))
}

/// Fetch the authenticated user's identity from Onshape and verify allowlist.
async fn fetch_and_verify_user(
    http_client: &reqwest::Client,
    access_token: &str,
    allowed_users: &HashSet<String>,
) -> Result<SessionInfo, (http::StatusCode, String)> {
    eprintln!("[oauth] callback: fetching user identity from Onshape");

    let session_info: SessionInfo = http_client
        .get("https://cad.onshape.com/api/v10/users/sessioninfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| {
            eprintln!("[oauth] callback: failed to fetch user info: {e}");
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to fetch user info: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            eprintln!("[oauth] callback: failed to parse user info: {e}");
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to parse user info: {e}"),
            )
        })?;

    eprintln!(
        "[oauth] callback: Onshape user id={} name={:?}",
        session_info.id, session_info.name
    );

    if !allowed_users.contains(&session_info.id) {
        eprintln!(
            "[oauth] callback: user {} not in allowlist, rejecting",
            session_info.id
        );
        return Err((
            http::StatusCode::FORBIDDEN,
            format!(
                "User {} is not authorized to use this server. \
                 Send this ID to the server administrator to request access. \
                 You can also find your Onshape user ID at \
                 https://cad.onshape.com/api/v10/users/sessioninfo",
                session_info.id
            ),
        ));
    }

    eprintln!(
        "[oauth] callback: user {} is on the allowlist",
        session_info.id
    );
    Ok(session_info)
}

async fn onshape_callback(
    State(state): State<Arc<OAuthServerState>>,
    Query(params): Query<CallbackParams>,
) -> Result<Redirect, (http::StatusCode, String)> {
    eprintln!("[oauth] callback: received Onshape redirect");

    // Check for OAuth errors from Onshape.
    if let Some(error) = &params.error {
        eprintln!("[oauth] callback: Onshape returned error: {error}");
        return Err((
            http::StatusCode::FORBIDDEN,
            format!("Onshape authorization denied: {error}"),
        ));
    }

    let Some(onshape_code) = params.code else {
        return Err((
            http::StatusCode::BAD_REQUEST,
            "missing authorization code".to_string(),
        ));
    };

    let Some(csrf_state) = params.state else {
        return Err((
            http::StatusCode::BAD_REQUEST,
            "missing state parameter".to_string(),
        ));
    };

    // Look up and consume the pending auth state.
    let Some(pending) = state.pending_auth.write().await.remove(&csrf_state) else {
        eprintln!("[oauth] callback: unknown or expired CSRF state");
        return Err((
            http::StatusCode::BAD_REQUEST,
            "unknown or expired state".to_string(),
        ));
    };

    eprintln!(
        "[oauth] callback: matched pending auth for client_id={}",
        pending.client_id
    );

    // Exchange Onshape code for tokens and fetch user identity.
    let (token_response, http_client) =
        exchange_onshape_code(&state, onshape_code, pending.onshape_pkce_verifier).await?;
    let access_token = token_response.access_token().secret().clone();
    let session_info =
        fetch_and_verify_user(&http_client, &access_token, &state.allowed_users).await?;

    // Compute expires_at from the token response.
    let now = chrono::Utc::now();
    let expires_at = token_response
        .expires_in()
        .and_then(|d| chrono::Duration::from_std(d).ok())
        .map(|d| now + d);

    // Store Onshape tokens for this user.
    let refresh_token = token_response
        .refresh_token()
        .map(|t| t.secret().clone())
        .unwrap_or_default();
    state.user_tokens.write().await.insert(
        session_info.id.clone(),
        UserOnshapeTokens::new(
            SecretString::from(access_token),
            SecretString::from(refresh_token),
            expires_at,
        ),
    );

    // Issue an MCP authorization code.
    let mcp_code = random_hex(32);
    state.auth_codes.write().await.insert(
        mcp_code.clone(),
        IssuedAuthCode {
            client_id: pending.client_id,
            redirect_uri: pending.redirect_uri.clone(),
            pkce_code_challenge: pending.pkce_code_challenge,
            user_id: session_info.id.clone(),
            created_at: chrono::Utc::now(),
        },
    );

    eprintln!(
        "[oauth] callback: issued MCP auth code for user {}, redirecting to MCP client",
        session_info.id
    );

    // Redirect back to the MCP client with the authorization code.
    let mut redirect = url::Url::parse(&pending.redirect_uri).map_err(|e| {
        (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid redirect URI: {e}"),
        )
    })?;
    {
        let mut qp = redirect.query_pairs_mut();
        qp.append_pair("code", &mcp_code);
        if let Some(ref state) = pending.mcp_state {
            qp.append_pair("state", state);
        }
    }

    Ok(Redirect::to(redirect.as_str()))
}

// ============================================================================
// Token Endpoint
// ============================================================================

/// Request body for `POST /oauth/token`.
#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

/// Response for `POST /oauth/token`.
#[derive(Debug, Serialize)]
struct TokenResponseBody {
    access_token: String,
    token_type: String,
    expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

async fn token_endpoint(
    State(state): State<Arc<OAuthServerState>>,
    axum::Form(req): axum::Form<TokenRequest>,
) -> Result<Json<TokenResponseBody>, (http::StatusCode, Json<serde_json::Value>)> {
    eprintln!(
        "[oauth] token: grant_type={} client_id={:?}",
        req.grant_type,
        req.client_id.as_deref().unwrap_or("<none>")
    );
    match req.grant_type.as_str() {
        "authorization_code" => handle_auth_code_grant(&state, &req).await,
        "refresh_token" => handle_refresh_token_grant(&state, &req).await,
        _ => Err(token_error(
            "unsupported_grant_type",
            "Only authorization_code and refresh_token are supported",
        )),
    }
}

async fn handle_auth_code_grant(
    state: &OAuthServerState,
    req: &TokenRequest,
) -> Result<Json<TokenResponseBody>, (http::StatusCode, Json<serde_json::Value>)> {
    use base64::Engine;
    use sha2::Digest;

    let Some(ref code) = req.code else {
        return Err(token_error("invalid_request", "missing code"));
    };

    // Look up and consume the authorization code.
    let Some(issued_code) = state.auth_codes.write().await.remove(code.as_str()) else {
        return Err(token_error("invalid_grant", "unknown or expired code"));
    };

    // Reject expired authorization codes (RFC 6749 §4.1.2).
    if chrono::Utc::now() > issued_code.created_at + chrono::Duration::seconds(AUTH_CODE_TTL_SECS) {
        return Err(token_error("invalid_grant", "unknown or expired code"));
    }

    // Validate client_id.
    if req.client_id.as_deref() != Some(&issued_code.client_id) {
        return Err(token_error("invalid_client", "client_id mismatch"));
    }

    // Enforce client authentication (client_secret_post).
    let Some(ref provided_secret) = req.client_secret else {
        return Err(token_error("invalid_client", "missing client_secret"));
    };
    let clients = state.clients.read().await;
    let Some(registered) = clients.get(&issued_code.client_id) else {
        return Err(token_error("invalid_client", "unknown client_id"));
    };
    if provided_secret != &registered.client_secret {
        return Err(token_error("invalid_client", "invalid client_secret"));
    }
    drop(clients);

    // Validate redirect_uri.
    if req.redirect_uri.as_deref() != Some(&issued_code.redirect_uri) {
        return Err(token_error("invalid_grant", "redirect_uri mismatch"));
    }

    // Validate PKCE if the client provided a code_challenge during authorization.
    if let Some(ref original_challenge) = issued_code.pkce_code_challenge {
        let Some(ref verifier) = req.code_verifier else {
            return Err(token_error("invalid_grant", "missing code_verifier"));
        };
        // Verify S256: SHA256(verifier) base64url-encoded == original challenge.
        let computed = sha2::Sha256::digest(verifier.as_bytes());
        let computed_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(computed);
        if computed_challenge != *original_challenge {
            return Err(token_error("invalid_grant", "PKCE verification failed"));
        }
    }

    // Issue the MCP access token.
    let access_token = random_hex(32);
    let mcp_refresh_token = random_hex(32);
    let now = chrono::Utc::now();
    let expires_at = now + chrono::Duration::seconds(TOKEN_LIFETIME_SECS);

    // Acquire both write guards in a fixed order (tokens → refresh_tokens)
    // so the two replacements are atomic with respect to concurrent readers.
    {
        let mut tokens = state.tokens.write().await;
        let mut refresh_tokens = state.refresh_tokens.write().await;

        // Revoke any prior access tokens for this user+client before issuing a new one.
        replace_token(
            &mut tokens,
            access_token.clone(),
            IssuedToken {
                user_id: issued_code.user_id.clone(),
                client_id: issued_code.client_id.clone(),
                issued_at: now,
                expires_at,
            },
        );

        // Revoke any prior refresh tokens for this user+client, then store the new one.
        replace_token(
            &mut refresh_tokens,
            mcp_refresh_token.clone(),
            IssuedToken {
                user_id: issued_code.user_id,
                client_id: issued_code.client_id,
                issued_at: now,
                expires_at: now + chrono::Duration::days(30), // refresh tokens live longer
            },
        );
    }

    Ok(Json(TokenResponseBody {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: TOKEN_LIFETIME_SECS,
        refresh_token: Some(mcp_refresh_token),
    }))
}

async fn handle_refresh_token_grant(
    state: &OAuthServerState,
    req: &TokenRequest,
) -> Result<Json<TokenResponseBody>, (http::StatusCode, Json<serde_json::Value>)> {
    let Some(ref refresh_token) = req.refresh_token else {
        return Err(token_error("invalid_request", "missing refresh_token"));
    };

    // Enforce client authentication (client_secret_post).
    let Some(ref client_id) = req.client_id else {
        return Err(token_error("invalid_client", "missing client_id"));
    };
    let Some(ref provided_secret) = req.client_secret else {
        return Err(token_error("invalid_client", "missing client_secret"));
    };
    let clients = state.clients.read().await;
    let Some(registered) = clients.get(client_id.as_str()) else {
        return Err(token_error("invalid_client", "unknown client_id"));
    };
    if provided_secret != &registered.client_secret {
        return Err(token_error("invalid_client", "invalid client_secret"));
    }
    drop(clients);

    // Consume the old refresh token from the dedicated refresh token map.
    let Some(old_token) = state
        .refresh_tokens
        .write()
        .await
        .remove(refresh_token.as_str())
    else {
        return Err(token_error(
            "invalid_grant",
            "unknown or expired refresh_token",
        ));
    };

    if chrono::Utc::now() > old_token.expires_at {
        return Err(token_error("invalid_grant", "refresh_token expired"));
    }

    // Verify the refresh token was issued to this client.
    if *client_id != old_token.client_id {
        return Err(token_error(
            "invalid_grant",
            "refresh_token not bound to this client",
        ));
    }

    // Issue new access + refresh tokens.
    let new_access = random_hex(32);
    let new_refresh = random_hex(32);
    let now = chrono::Utc::now();
    let expires_at = now + chrono::Duration::seconds(TOKEN_LIFETIME_SECS);

    // Acquire both write guards in a fixed order (tokens → refresh_tokens)
    // so the two replacements are atomic with respect to concurrent readers.
    {
        let mut tokens = state.tokens.write().await;
        let mut refresh_tokens = state.refresh_tokens.write().await;

        // Revoke any prior access tokens for this user+client before issuing a new one.
        replace_token(
            &mut tokens,
            new_access.clone(),
            IssuedToken {
                user_id: old_token.user_id.clone(),
                client_id: client_id.clone(),
                issued_at: now,
                expires_at,
            },
        );
        // The old refresh token was already consumed via .remove() above;
        // retain() here catches any orphaned entries from prior flows.
        replace_token(
            &mut refresh_tokens,
            new_refresh.clone(),
            IssuedToken {
                user_id: old_token.user_id,
                client_id: client_id.clone(),
                issued_at: now,
                expires_at: now + chrono::Duration::days(30),
            },
        );
    }

    Ok(Json(TokenResponseBody {
        access_token: new_access,
        token_type: "Bearer".to_string(),
        expires_in: TOKEN_LIFETIME_SECS,
        refresh_token: Some(new_refresh),
    }))
}

fn token_error(error: &str, description: &str) -> (http::StatusCode, Json<serde_json::Value>) {
    (
        http::StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
}

// ============================================================================
// Auth Middleware
// ============================================================================

/// Build a 401 response with the required `WWW-Authenticate` header (RFC 6750).
fn unauthorized_response(error: &str, description: &str) -> axum::response::Response {
    (
        http::StatusCode::UNAUTHORIZED,
        [(
            http::header::WWW_AUTHENTICATE,
            format!("Bearer error=\"{error}\", error_description=\"{description}\""),
        )],
        description.to_string(),
    )
        .into_response()
}

/// Axum middleware that validates Bearer tokens on the MCP endpoint.
///
/// Extracts the `Authorization: Bearer <token>` header, validates it
/// against the OAuth server state, and inserts `UserContext` into the
/// request extensions.  Returns `WWW-Authenticate` on 401 per RFC 6750.
pub(crate) async fn auth_middleware(
    State(state): State<Arc<OAuthServerState>>,
    mut request: http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, axum::response::Response> {
    let method = request.method().clone();
    let uri = request.uri().clone();

    let auth_header = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let Some(auth_value) = auth_header else {
        eprintln!("[oauth] auth: {method} {uri} — missing Authorization header");
        return Err(unauthorized_response(
            "invalid_request",
            "Missing Authorization header",
        ));
    };

    // Parse scheme case-insensitively per RFC 9110 Section 11.1.
    let token = if auth_value.len() > 7 && auth_value[..7].eq_ignore_ascii_case("bearer ") {
        &auth_value[7..]
    } else {
        eprintln!("[oauth] auth: {method} {uri} — invalid Authorization header format");
        return Err(unauthorized_response(
            "invalid_request",
            "Invalid Authorization header format",
        ));
    };

    let Some(user_ctx) = state.validate_token(token).await else {
        eprintln!("[oauth] auth: {method} {uri} — invalid or expired token");
        return Err(unauthorized_response(
            "invalid_token",
            "Invalid or expired token",
        ));
    };

    eprintln!(
        "[oauth] auth: {method} {uri} — authenticated user {}",
        user_ctx.user_id
    );
    request.extensions_mut().insert(user_ctx);
    Ok(next.run(request).await)
}

// ============================================================================
// Router
// ============================================================================

/// Build the OAuth server router with all endpoints.
///
/// The returned router includes:
/// - `GET /health` — Health check (returns 200 OK)
/// - `GET /.well-known/oauth-protected-resource/mcp` — RFC 9728 (path-suffixed)
/// - `GET /.well-known/oauth-protected-resource` — RFC 9728 (fallback without suffix)
/// - `GET /.well-known/oauth-authorization-server` — RFC 8414
/// - `POST /oauth/register` — Dynamic Client Registration
/// - `GET /oauth/authorize` — Authorization endpoint
/// - `GET /oauth/callback` — Onshape callback
/// - `POST /oauth/token` — Token endpoint
///
/// CORS is applied to all OAuth router endpoints (not just metadata/token).
///
/// Per RFC 9728 Section 3, when the protected resource URL has a path
/// component (e.g. `https://example.com/mcp`), the well-known URI is
/// constructed by inserting `/.well-known/oauth-protected-resource`
/// after the authority, preserving the path suffix:
/// `https://example.com/.well-known/oauth-protected-resource/mcp`.
/// We serve both the path-suffixed and bare variants for robustness.
pub(crate) fn oauth_router(state: Arc<OAuthServerState>) -> Router {
    use tower_http::cors::{Any, CorsLayer};

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", routing::get(health))
        // RFC 9728: path-suffixed variant (matches resource path `/mcp`).
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            routing::get(protected_resource_metadata),
        )
        // RFC 9728: bare variant (some clients may omit the path suffix).
        .route(
            "/.well-known/oauth-protected-resource",
            routing::get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            routing::get(authorization_server_metadata),
        )
        .route("/oauth/register", routing::post(register_client))
        .route("/oauth/authorize", routing::get(authorize))
        .route("/oauth/callback", routing::get(onshape_callback))
        .route("/oauth/token", routing::post(token_endpoint))
        .layer(cors)
        .with_state(state)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::similar_names)]
mod tests {
    use super::*;
    use sha2::Digest as _;

    /// Helper: create a test `OAuthServerState` with a single allowed user.
    fn test_state() -> OAuthServerState {
        OAuthServerState::new(
            url::Url::parse("https://example.com").expect("valid test URL"),
            "onshape-client-id".to_string(),
            SecretString::from("onshape-client-secret"),
            vec!["allowed-user-1".to_string()],
        )
    }

    /// Helper: register a client and return (`client_id`, `client_secret`).
    async fn register_test_client(state: &OAuthServerState) -> (String, String) {
        let client_id = random_hex(16);
        let client_secret = random_hex(32);
        state.clients.write().await.insert(
            client_id.clone(),
            RegisteredClient {
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                redirect_uris: vec!["https://example.com/callback".to_string()],
            },
        );
        (client_id, client_secret)
    }

    /// Helper: insert an access token and return the token string.
    async fn insert_access_token(
        state: &OAuthServerState,
        user_id: &str,
        client_id: &str,
    ) -> String {
        let token = random_hex(32);
        let now = chrono::Utc::now();
        state.tokens.write().await.insert(
            token.clone(),
            IssuedToken {
                user_id: user_id.to_string(),
                client_id: client_id.to_string(),
                issued_at: now,
                expires_at: now + chrono::Duration::seconds(TOKEN_LIFETIME_SECS),
            },
        );
        // Also insert user tokens so validate_token can find them.
        state.user_tokens.write().await.insert(
            user_id.to_string(),
            UserOnshapeTokens::new(
                SecretString::from("onshape-access-token"),
                SecretString::from("onshape-refresh-token"),
                Some(now + chrono::Duration::hours(1)),
            ),
        );
        token
    }

    // ================================================================
    // validate_token tests
    // ================================================================

    #[tokio::test]
    async fn validate_token_accepts_valid_access_token() {
        let state = test_state();
        let (client_id, _) = register_test_client(&state).await;
        let token = insert_access_token(&state, "allowed-user-1", &client_id).await;

        let result = state.validate_token(&token).await;
        assert!(result.is_some());
        let ctx = result.expect("should be Some");
        assert_eq!(ctx.user_id, "allowed-user-1");
    }

    #[tokio::test]
    async fn validate_token_rejects_expired_token() {
        let state = test_state();
        let token = random_hex(32);
        let now = chrono::Utc::now();
        state.tokens.write().await.insert(
            token.clone(),
            IssuedToken {
                user_id: "allowed-user-1".to_string(),
                client_id: "some-client".to_string(),
                issued_at: now - chrono::Duration::hours(2),
                expires_at: now - chrono::Duration::hours(1), // expired
            },
        );

        let result = state.validate_token(&token).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn validate_token_rejects_unknown_token() {
        let state = test_state();
        let result = state.validate_token("nonexistent-token").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn validate_token_rejects_refresh_token() {
        // Refresh tokens are stored in a separate map, so they should
        // never be accepted as bearer tokens.
        let state = test_state();
        let refresh_token = random_hex(32);
        let now = chrono::Utc::now();
        state.refresh_tokens.write().await.insert(
            refresh_token.clone(),
            IssuedToken {
                user_id: "allowed-user-1".to_string(),
                client_id: "some-client".to_string(),
                issued_at: now,
                expires_at: now + chrono::Duration::days(30),
            },
        );

        // The refresh token should NOT be findable via validate_token.
        let result = state.validate_token(&refresh_token).await;
        assert!(result.is_none());

        // Even with the old "refresh:" prefix convention, it should not work.
        let prefixed = format!("refresh:{refresh_token}");
        let result = state.validate_token(&prefixed).await;
        assert!(result.is_none());
    }

    // ================================================================
    // DCR validation tests
    // ================================================================

    #[tokio::test]
    async fn dcr_rejects_empty_redirect_uris() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: Some("test".to_string()),
            redirect_uris: vec![],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dcr_rejects_invalid_redirect_uri_syntax() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["not a url".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        assert!(result.is_err());
        let (status, body) = result.expect_err("should reject invalid redirect URI");
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(body.0["error"], "invalid_client_metadata");
    }

    #[tokio::test]
    async fn dcr_rejects_mixed_valid_and_invalid_redirect_uris() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec![
                "https://example.com/cb".to_string(),
                "://broken".to_string(),
            ],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        assert!(result.is_err());
        let (status, body) = result.expect_err("should reject mixed valid/invalid redirect URIs");
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(body.0["error"], "invalid_client_metadata");
    }

    #[tokio::test]
    async fn dcr_rejects_http_non_localhost_redirect_uri() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["http://example.com/cb".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        let (status, body) = result.expect_err("should reject http:// to non-localhost");
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(body.0["error"], "invalid_client_metadata");
    }

    #[tokio::test]
    async fn dcr_rejects_ftp_redirect_uri() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["ftp://example.com/cb".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        let (status, body) = result.expect_err("should reject ftp:// redirect URI");
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(body.0["error"], "invalid_client_metadata");
    }

    #[tokio::test]
    async fn dcr_accepts_http_localhost_redirect_uri() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["http://localhost:8080/cb".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        let _ = result.expect("http://localhost should be accepted");
    }

    #[tokio::test]
    async fn dcr_accepts_http_127_0_0_1_redirect_uri() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["http://127.0.0.1:8080/cb".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        let _ = result.expect("http://127.0.0.1 should be accepted");
    }

    #[tokio::test]
    async fn dcr_accepts_http_alternate_ipv4_loopback_redirect_uri() {
        // The entire 127.0.0.0/8 range is loopback per RFC 1122.  The MCP spec
        // says "localhost URLs", and RFC 8252 §7.3 says "loopback IP literal"
        // without restricting to 127.0.0.1 specifically.
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["http://127.0.0.2:8080/cb".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        let _ = result.expect("http://127.0.0.2 (loopback) should be accepted");
    }

    #[tokio::test]
    async fn dcr_accepts_http_ipv6_loopback_redirect_uri() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["http://[::1]:8080/cb".to_string()],
            grant_types: vec![],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        let _ = result.expect("http://[::1] should be accepted");
    }

    #[tokio::test]
    async fn dcr_rejects_unsupported_grant_type() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["https://example.com/cb".to_string()],
            grant_types: vec!["implicit".to_string()],
            response_types: vec![],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dcr_rejects_unsupported_response_type() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: None,
            redirect_uris: vec!["https://example.com/cb".to_string()],
            grant_types: vec![],
            response_types: vec!["token".to_string()],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dcr_accepts_valid_registration() {
        let state = Arc::new(test_state());
        let req = RegisterRequest {
            client_name: Some("My App".to_string()),
            redirect_uris: vec!["https://example.com/cb".to_string()],
            grant_types: vec!["authorization_code".to_string()],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: None,
        };

        let result = register_client(State(state.clone()), Json(req)).await;
        assert!(result.is_ok());

        let response = result.expect("should be Ok");
        assert!(!response.client_id.is_empty());
        assert!(!response.client_secret.is_empty());
        assert_eq!(response.token_endpoint_auth_method, "client_secret_post");
    }

    // ================================================================
    // Token endpoint tests (auth code grant)
    // ================================================================

    #[tokio::test]
    async fn auth_code_grant_rejects_missing_client_secret() {
        let state = test_state();
        let (client_id, _client_secret) = register_test_client(&state).await;

        // Insert an auth code.
        let code = random_hex(32);
        state.auth_codes.write().await.insert(
            code.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: None,
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now(),
            },
        );

        let req = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id),
            client_secret: None, // missing!
            code_verifier: None,
            refresh_token: None,
        };

        let result = handle_auth_code_grant(&state, &req).await;
        assert!(result.is_err());
        let (status, json) = result.expect_err("should be Err");
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(json.0["error"], "invalid_client");
    }

    #[tokio::test]
    async fn auth_code_grant_rejects_wrong_client_secret() {
        let state = test_state();
        let (client_id, _client_secret) = register_test_client(&state).await;

        let code = random_hex(32);
        state.auth_codes.write().await.insert(
            code.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: None,
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now(),
            },
        );

        let req = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id),
            client_secret: Some("wrong-secret".to_string()),
            code_verifier: None,
            refresh_token: None,
        };

        let result = handle_auth_code_grant(&state, &req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn auth_code_grant_validates_pkce_s256() {
        use base64::Engine;

        let state = test_state();
        let (client_id, client_secret) = register_test_client(&state).await;

        // Create a PKCE challenge/verifier pair.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let digest = sha2::Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

        let code = random_hex(32);
        state.auth_codes.write().await.insert(
            code.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: Some(challenge),
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now(),
            },
        );

        // Insert user tokens so the token issuance can succeed.
        state.user_tokens.write().await.insert(
            "allowed-user-1".to_string(),
            UserOnshapeTokens::new(
                SecretString::from("onshape-at"),
                SecretString::from("onshape-rt"),
                None,
            ),
        );

        // Correct verifier should succeed.
        let req = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id.clone()),
            client_secret: Some(client_secret.clone()),
            code_verifier: Some(verifier.to_string()),
            refresh_token: None,
        };

        let result = handle_auth_code_grant(&state, &req).await;
        assert!(result.is_ok());
        let body = result.expect("should be Ok");
        assert_eq!(body.token_type, "Bearer");
        assert!(!body.access_token.is_empty());
        assert!(body.refresh_token.is_some());
    }

    #[tokio::test]
    async fn auth_code_grant_rejects_wrong_pkce_verifier() {
        use base64::Engine;

        let state = test_state();
        let (client_id, client_secret) = register_test_client(&state).await;

        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(b"correct-verifier"));

        let code = random_hex(32);
        state.auth_codes.write().await.insert(
            code.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: Some(challenge),
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now(),
            },
        );

        let req = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id),
            client_secret: Some(client_secret),
            code_verifier: Some("wrong-verifier".to_string()),
            refresh_token: None,
        };

        let result = handle_auth_code_grant(&state, &req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn auth_code_grant_rejects_expired_code() {
        let state = test_state();
        let (client_id, client_secret) = register_test_client(&state).await;

        let code = random_hex(32);
        state.auth_codes.write().await.insert(
            code.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: None,
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now() - chrono::Duration::seconds(AUTH_CODE_TTL_SECS + 1),
            },
        );

        let req = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id),
            client_secret: Some(client_secret),
            code_verifier: None,
            refresh_token: None,
        };

        let result = handle_auth_code_grant(&state, &req).await;
        assert!(result.is_err());
        let (status, json) = result.expect_err("should be Err");
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(json.0["error"], "invalid_grant");
    }

    // ================================================================
    // Refresh token grant tests
    // ================================================================

    #[tokio::test]
    async fn refresh_grant_rejects_missing_client_credentials() {
        let state = test_state();
        let req = TokenRequest {
            grant_type: "refresh_token".to_string(),
            code: None,
            redirect_uri: None,
            client_id: None, // missing
            client_secret: None,
            code_verifier: None,
            refresh_token: Some("some-refresh-token".to_string()),
        };

        let result = handle_refresh_token_grant(&state, &req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn refresh_grant_rejects_wrong_client_binding() {
        let state = test_state();
        let (client_a_id, client_a_secret) = register_test_client(&state).await;
        let (client_b_id, client_b_secret) = register_test_client(&state).await;

        // Insert a refresh token bound to client A.
        let refresh_token = random_hex(32);
        let now = chrono::Utc::now();
        state.refresh_tokens.write().await.insert(
            refresh_token.clone(),
            IssuedToken {
                user_id: "allowed-user-1".to_string(),
                client_id: client_a_id.clone(),
                issued_at: now,
                expires_at: now + chrono::Duration::days(30),
            },
        );
        // Need user tokens for the lookup.
        state.user_tokens.write().await.insert(
            "allowed-user-1".to_string(),
            UserOnshapeTokens::new(SecretString::from("at"), SecretString::from("rt"), None),
        );

        // Client B should NOT be able to use client A's refresh token.
        let req = TokenRequest {
            grant_type: "refresh_token".to_string(),
            code: None,
            redirect_uri: None,
            client_id: Some(client_b_id),
            client_secret: Some(client_b_secret),
            code_verifier: None,
            refresh_token: Some(refresh_token),
        };

        let result = handle_refresh_token_grant(&state, &req).await;
        assert!(result.is_err());

        // Suppress unused variable warnings.
        let _ = (client_a_secret, client_a_id);
    }

    #[tokio::test]
    async fn refresh_grant_succeeds_with_correct_client() {
        let state = test_state();
        let (client_id, client_secret) = register_test_client(&state).await;

        let refresh_token = random_hex(32);
        let now = chrono::Utc::now();
        state.refresh_tokens.write().await.insert(
            refresh_token.clone(),
            IssuedToken {
                user_id: "allowed-user-1".to_string(),
                client_id: client_id.clone(),
                issued_at: now,
                expires_at: now + chrono::Duration::days(30),
            },
        );
        state.user_tokens.write().await.insert(
            "allowed-user-1".to_string(),
            UserOnshapeTokens::new(SecretString::from("at"), SecretString::from("rt"), None),
        );

        let req = TokenRequest {
            grant_type: "refresh_token".to_string(),
            code: None,
            redirect_uri: None,
            client_id: Some(client_id),
            client_secret: Some(client_secret),
            code_verifier: None,
            refresh_token: Some(refresh_token.clone()),
        };

        let result = handle_refresh_token_grant(&state, &req).await;
        assert!(result.is_ok());

        // The old refresh token should be consumed (single-use).
        assert!(
            !state
                .refresh_tokens
                .read()
                .await
                .contains_key(&refresh_token)
        );
    }

    // ================================================================
    // Token revocation on grant tests
    // ================================================================

    #[tokio::test]
    async fn refresh_grant_revokes_old_access_token() {
        let state = test_state();
        let (client_id, client_secret) = register_test_client(&state).await;

        // Insert an existing access token for this user+client.
        let old_access = insert_access_token(&state, "allowed-user-1", &client_id).await;

        // Insert a refresh token for the same user+client.
        let refresh_token = random_hex(32);
        let now = chrono::Utc::now();
        state.refresh_tokens.write().await.insert(
            refresh_token.clone(),
            IssuedToken {
                user_id: "allowed-user-1".to_string(),
                client_id: client_id.clone(),
                issued_at: now,
                expires_at: now + chrono::Duration::days(30),
            },
        );

        // Perform the refresh grant.
        let expected_client_id = client_id.clone();
        let req = TokenRequest {
            grant_type: "refresh_token".to_string(),
            code: None,
            redirect_uri: None,
            client_id: Some(client_id),
            client_secret: Some(client_secret),
            code_verifier: None,
            refresh_token: Some(refresh_token),
        };

        let result = handle_refresh_token_grant(&state, &req).await;
        assert!(result.is_ok());

        // The old access token should have been revoked.
        assert!(
            !state.tokens.read().await.contains_key(&old_access),
            "old access token should be revoked after refresh"
        );

        // A new access token should exist (exactly one for this user+client).
        assert_eq!(
            state
                .tokens
                .read()
                .await
                .values()
                .filter(|t| t.user_id == "allowed-user-1" && t.client_id == expected_client_id)
                .count(),
            1,
            "exactly one access token should exist after refresh"
        );
    }

    #[tokio::test]
    async fn auth_code_grant_revokes_old_tokens_for_same_client() {
        let state = test_state();
        let (client_id, client_secret) = register_test_client(&state).await;

        // Set up user tokens so the grant can succeed.
        state.user_tokens.write().await.insert(
            "allowed-user-1".to_string(),
            UserOnshapeTokens::new(
                SecretString::from("onshape-at"),
                SecretString::from("onshape-rt"),
                None,
            ),
        );

        // First auth code grant — issues initial tokens.
        let code1 = random_hex(32);
        state.auth_codes.write().await.insert(
            code1.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: None,
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now(),
            },
        );

        let req1 = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code1),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id.clone()),
            client_secret: Some(client_secret.clone()),
            code_verifier: None,
            refresh_token: None,
        };

        let result1 = handle_auth_code_grant(&state, &req1).await;
        assert!(result1.is_ok());
        let body1 = result1.expect("first grant should succeed");
        let first_access = body1.access_token.clone();
        let first_refresh = body1
            .refresh_token
            .clone()
            .expect("should have refresh token");

        // Verify the first tokens exist.
        assert!(state.tokens.read().await.contains_key(&first_access));
        assert!(
            state
                .refresh_tokens
                .read()
                .await
                .contains_key(&first_refresh)
        );

        // Second auth code grant for the same user+client — should revoke the first tokens.
        let code2 = random_hex(32);
        state.auth_codes.write().await.insert(
            code2.clone(),
            IssuedAuthCode {
                client_id: client_id.clone(),
                redirect_uri: "https://example.com/callback".to_string(),
                pkce_code_challenge: None,
                user_id: "allowed-user-1".to_string(),
                created_at: chrono::Utc::now(),
            },
        );

        let req2 = TokenRequest {
            grant_type: "authorization_code".to_string(),
            code: Some(code2),
            redirect_uri: Some("https://example.com/callback".to_string()),
            client_id: Some(client_id.clone()),
            client_secret: Some(client_secret),
            code_verifier: None,
            refresh_token: None,
        };

        let result2 = handle_auth_code_grant(&state, &req2).await;
        assert!(result2.is_ok());

        // The first access and refresh tokens should be revoked.
        assert!(
            !state.tokens.read().await.contains_key(&first_access),
            "first access token should be revoked after re-auth"
        );
        assert!(
            !state
                .refresh_tokens
                .read()
                .await
                .contains_key(&first_refresh),
            "first refresh token should be revoked after re-auth"
        );

        // Exactly one access token and one refresh token should remain.
        let access_count = state
            .tokens
            .read()
            .await
            .values()
            .filter(|t| t.user_id == "allowed-user-1" && t.client_id == client_id)
            .count();
        let refresh_count = state
            .refresh_tokens
            .read()
            .await
            .values()
            .filter(|t| t.user_id == "allowed-user-1" && t.client_id == client_id)
            .count();
        assert_eq!(access_count, 1, "exactly one access token after re-auth");
        assert_eq!(refresh_count, 1, "exactly one refresh token after re-auth");
    }
}
