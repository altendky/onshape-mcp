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
use rand::Rng;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use onshape_client_core::oauth::onshape_oauth_client;

// ============================================================================
// Types
// ============================================================================

/// Onshape tokens stored for an authenticated user.
#[derive(Clone, Debug)]
pub(crate) struct UserOnshapeTokens {
    pub access_token: String,
    /// Kept for future per-user token refresh.
    #[allow(dead_code)]
    pub refresh_token: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Context inserted into HTTP request extensions by the auth middleware.
///
/// Accessible in `call_tool()` via `request::Parts::extensions`.
#[derive(Clone, Debug)]
pub(crate) struct UserContext {
    /// Onshape user ID (used for logging and future per-user management).
    #[allow(dead_code)]
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
    /// PKCE code verifier for the MCP client's auth request.
    pkce_verifier: Option<String>,
    /// The CSRF state token from the MCP client's auth request.
    mcp_state: String,
    /// PKCE verifier for the Onshape leg of the flow.
    onshape_pkce_verifier: PkceCodeVerifier,
}

/// A dynamically registered MCP client.
#[derive(Debug, Clone)]
struct RegisteredClient {
    #[allow(dead_code)]
    client_id: String,
    #[allow(dead_code)]
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
    /// PKCE code verifier from the client's authorization request.
    pkce_verifier: Option<String>,
    /// Onshape user ID associated with this code.
    user_id: String,
}

/// An issued MCP access token → user mapping.
#[derive(Debug, Clone)]
struct IssuedToken {
    /// Onshape user ID.
    user_id: String,
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
    /// User Onshape tokens (keyed by Onshape user ID).
    pub(crate) user_tokens: RwLock<HashMap<String, UserOnshapeTokens>>,
    /// Allowlist of Onshape user IDs.
    allowed_users: HashSet<String>,
    /// Onshape OAuth app client ID (operator's app).
    onshape_client_id: String,
    /// Onshape OAuth app client secret (operator's app).
    onshape_client_secret: SecretString,
    /// Public URL of this MCP server.
    public_url: String,
}

/// MCP access token lifetime (1 hour, matching Onshape).
const TOKEN_LIFETIME_SECS: i64 = 3600;

// ============================================================================
// State Construction
// ============================================================================

impl OAuthServerState {
    /// Create a new OAuth server state.
    pub(crate) fn new(
        public_url: String,
        onshape_client_id: String,
        onshape_client_secret: SecretString,
        allowed_user_ids: Vec<String>,
    ) -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            pending_auth: RwLock::new(HashMap::new()),
            auth_codes: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
            user_tokens: RwLock::new(HashMap::new()),
            allowed_users: allowed_user_ids.into_iter().collect(),
            onshape_client_id,
            onshape_client_secret,
            public_url,
        }
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
}

// ============================================================================
// Helper: Random Token Generation
// ============================================================================

/// Generate a cryptographically random hex string.
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

/// RFC 9728: Protected Resource Metadata.
///
/// `GET /.well-known/oauth-protected-resource`
async fn protected_resource_metadata(
    State(state): State<Arc<OAuthServerState>>,
) -> impl IntoResponse {
    Json(serde_json::json!({
        "resource": format!("{}/mcp", state.public_url),
        "authorization_servers": [state.public_url],
        "bearer_methods_supported": ["header"],
    }))
}

/// RFC 8414: Authorization Server Metadata.
///
/// `GET /.well-known/oauth-authorization-server`
async fn authorization_server_metadata(
    State(state): State<Arc<OAuthServerState>>,
) -> impl IntoResponse {
    let base = &state.public_url;
    Json(serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
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
#[derive(Serialize)]
struct RegisterResponse {
    client_id: String,
    client_secret: String,
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
}

async fn register_client(
    State(state): State<Arc<OAuthServerState>>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let client_id = random_hex(16);
    let client_secret = random_hex(32);

    let grant_types = if req.grant_types.is_empty() {
        vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ]
    } else {
        req.grant_types
    };

    let response_types = if req.response_types.is_empty() {
        vec!["code".to_string()]
    } else {
        req.response_types
    };

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

    Json(RegisterResponse {
        client_id,
        client_secret,
        client_name: req.client_name,
        redirect_uris: req.redirect_uris,
        grant_types,
        response_types,
        token_endpoint_auth_method: "client_secret_post".to_string(),
    })
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
    #[allow(dead_code)]
    code_challenge_method: Option<String>,
    #[allow(dead_code)]
    scope: Option<String>,
}

async fn authorize(
    State(state): State<Arc<OAuthServerState>>,
    Query(params): Query<AuthorizeParams>,
) -> Result<Redirect, (http::StatusCode, String)> {
    // Validate response_type.
    if params.response_type != "code" {
        return Err((
            http::StatusCode::BAD_REQUEST,
            "unsupported response_type".to_string(),
        ));
    }

    // Validate client_id.
    let clients = state.clients.read().await;
    let Some(client) = clients.get(&params.client_id) else {
        return Err((
            http::StatusCode::BAD_REQUEST,
            "unknown client_id".to_string(),
        ));
    };

    // Validate redirect_uri.
    if !client.redirect_uris.contains(&params.redirect_uri) {
        return Err((
            http::StatusCode::BAD_REQUEST,
            "redirect_uri not registered".to_string(),
        ));
    }
    drop(clients);

    // Store the MCP client's PKCE verifier (if provided) for later validation.
    let pkce_verifier = params.code_challenge.clone();

    // Generate Onshape OAuth parameters.
    let (onshape_pkce_challenge, onshape_pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let onshape_csrf = CsrfToken::new_random();

    // Store pending auth state keyed by the Onshape CSRF token.
    let pending = PendingAuth {
        client_id: params.client_id.clone(),
        redirect_uri: params.redirect_uri.clone(),
        pkce_verifier,
        mcp_state: params.state.clone().unwrap_or_default(),
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
    let callback_url = format!("{}/oauth/callback", state.public_url);
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
    let onshape_client = onshape_oauth_client(
        &state.onshape_client_id,
        state.onshape_client_secret.expose_secret(),
    );
    let callback_url = format!("{}/oauth/callback", state.public_url);
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
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("token exchange failed: {e}"),
            )
        })?;

    Ok((token_response, http_client))
}

/// Fetch the authenticated user's identity from Onshape and verify allowlist.
async fn fetch_and_verify_user(
    http_client: &reqwest::Client,
    access_token: &str,
    allowed_users: &HashSet<String>,
) -> Result<SessionInfo, (http::StatusCode, String)> {
    let session_info: SessionInfo = http_client
        .get("https://cad.onshape.com/api/v10/users/sessioninfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| {
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to fetch user info: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to parse user info: {e}"),
            )
        })?;

    if !allowed_users.contains(&session_info.id) {
        return Err((
            http::StatusCode::FORBIDDEN,
            format!(
                "User {} is not authorized to use this server. \
                 Contact the server administrator to be added to the allowlist.",
                session_info.id
            ),
        ));
    }

    Ok(session_info)
}

async fn onshape_callback(
    State(state): State<Arc<OAuthServerState>>,
    Query(params): Query<CallbackParams>,
) -> Result<Redirect, (http::StatusCode, String)> {
    // Check for OAuth errors from Onshape.
    if let Some(error) = &params.error {
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
        return Err((
            http::StatusCode::BAD_REQUEST,
            "unknown or expired state".to_string(),
        ));
    };

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
        UserOnshapeTokens {
            access_token,
            refresh_token,
            expires_at,
        },
    );

    // Issue an MCP authorization code.
    let mcp_code = random_hex(32);
    state.auth_codes.write().await.insert(
        mcp_code.clone(),
        IssuedAuthCode {
            client_id: pending.client_id,
            redirect_uri: pending.redirect_uri.clone(),
            pkce_verifier: pending.pkce_verifier,
            user_id: session_info.id,
        },
    );

    // Redirect back to the MCP client with the authorization code.
    let mut redirect = url::Url::parse(&pending.redirect_uri).map_err(|e| {
        (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid redirect URI: {e}"),
        )
    })?;
    redirect
        .query_pairs_mut()
        .append_pair("code", &mcp_code)
        .append_pair("state", &pending.mcp_state);

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
    #[allow(dead_code)]
    client_secret: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

/// Response for `POST /oauth/token`.
#[derive(Serialize)]
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

    // Validate client_id.
    if req.client_id.as_deref() != Some(&issued_code.client_id) {
        return Err(token_error("invalid_client", "client_id mismatch"));
    }

    // Validate redirect_uri.
    if req.redirect_uri.as_deref() != Some(&issued_code.redirect_uri) {
        return Err(token_error("invalid_grant", "redirect_uri mismatch"));
    }

    // Validate PKCE if the client provided a code_challenge during authorization.
    if let Some(ref original_challenge) = issued_code.pkce_verifier {
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

    state.tokens.write().await.insert(
        access_token.clone(),
        IssuedToken {
            user_id: issued_code.user_id.clone(),
            issued_at: now,
            expires_at,
        },
    );

    // Store refresh token → user mapping (reuse the tokens map with a prefix).
    state.tokens.write().await.insert(
        format!("refresh:{mcp_refresh_token}"),
        IssuedToken {
            user_id: issued_code.user_id,
            issued_at: now,
            expires_at: now + chrono::Duration::days(30), // refresh tokens live longer
        },
    );

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

    let key = format!("refresh:{refresh_token}");

    // Consume the old refresh token.
    let Some(old_token) = state.tokens.write().await.remove(&key) else {
        return Err(token_error(
            "invalid_grant",
            "unknown or expired refresh_token",
        ));
    };

    if chrono::Utc::now() > old_token.expires_at {
        return Err(token_error("invalid_grant", "refresh_token expired"));
    }

    // Issue new access + refresh tokens.
    let new_access = random_hex(32);
    let new_refresh = random_hex(32);
    let now = chrono::Utc::now();
    let expires_at = now + chrono::Duration::seconds(TOKEN_LIFETIME_SECS);

    state.tokens.write().await.insert(
        new_access.clone(),
        IssuedToken {
            user_id: old_token.user_id.clone(),
            issued_at: now,
            expires_at,
        },
    );
    state.tokens.write().await.insert(
        format!("refresh:{new_refresh}"),
        IssuedToken {
            user_id: old_token.user_id,
            issued_at: now,
            expires_at: now + chrono::Duration::days(30),
        },
    );

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

/// Axum middleware that validates Bearer tokens on the MCP endpoint.
///
/// Extracts the `Authorization: Bearer <token>` header, validates it
/// against the OAuth server state, and inserts `UserContext` into the
/// request extensions.
pub(crate) async fn auth_middleware(
    State(state): State<Arc<OAuthServerState>>,
    mut request: http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, (http::StatusCode, String)> {
    let auth_header = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let Some(auth_value) = auth_header else {
        return Err((
            http::StatusCode::UNAUTHORIZED,
            "Missing Authorization header".to_string(),
        ));
    };

    let Some(token) = auth_value.strip_prefix("Bearer ") else {
        return Err((
            http::StatusCode::UNAUTHORIZED,
            "Invalid Authorization header format".to_string(),
        ));
    };

    let Some(user_ctx) = state.validate_token(token).await else {
        return Err((
            http::StatusCode::UNAUTHORIZED,
            "Invalid or expired token".to_string(),
        ));
    };

    request.extensions_mut().insert(user_ctx);
    Ok(next.run(request).await)
}

// ============================================================================
// Router
// ============================================================================

/// Build the OAuth server router with all endpoints.
///
/// The returned router includes:
/// - `GET /.well-known/oauth-protected-resource` — RFC 9728
/// - `GET /.well-known/oauth-authorization-server` — RFC 8414
/// - `POST /oauth/register` — Dynamic Client Registration
/// - `GET /oauth/authorize` — Authorization endpoint
/// - `GET /oauth/callback` — Onshape callback
/// - `POST /oauth/token` — Token endpoint
///
/// CORS is applied to metadata and token endpoints.
pub(crate) fn oauth_router(state: Arc<OAuthServerState>) -> Router {
    use tower_http::cors::{Any, CorsLayer};

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
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
