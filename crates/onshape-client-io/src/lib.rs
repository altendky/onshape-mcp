//! HTTP client for the Onshape API.
//!
//! This crate provides the I/O layer for executing [`ApiRequest`]s against the
//! Onshape REST API. It handles URL construction, authentication headers,
//! timeouts, and transport error classification.
//!
//! # Architecture
//!
//! This is a thin async wrapper around `reqwest`. The pure request/response
//! types live in `onshape-client-core` (sans-IO); this crate only adds the
//! actual HTTP execution.

use std::sync::Arc;
use std::time::Duration;

use oauth2::AccessToken;
use onshape_client_core::auth::{
    Credentials, basic_authorization_header_value, bearer_authorization_header_value,
};
use onshape_client_core::request::{
    ApiRequest, ApiResponse, HttpMethod, RequestBody, ResponseBody,
};
use reqwest::Client;
use secrecy::ExposeSecret;

// ============================================================================
// Error Type
// ============================================================================

/// Transport-level errors from the HTTP client.
///
/// These represent failures to communicate with the Onshape API server.
/// HTTP error status codes (4xx, 5xx) are **not** errors at this layer —
/// they come back as [`ApiResponse`] with the appropriate status code.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Failed to establish a connection to the server.
    #[error("connection failed: {message}")]
    Connection { message: String },

    /// The request timed out.
    #[error("request timed out after {timeout:?}")]
    Timeout { timeout: Duration },

    /// Failed to build the HTTP request (e.g., invalid URL).
    #[error("failed to build request: {message}")]
    RequestBuild { message: String },

    /// Failed to read the response body.
    #[error("failed to read response body: {message}")]
    ResponseBody { message: String },

    /// An unexpected transport error.
    #[error("HTTP transport error: {message}")]
    Transport { message: String },
}

impl ClientError {
    /// Classify a `reqwest::Error` into a `ClientError`.
    fn from_reqwest(err: &reqwest::Error, timeout: Duration) -> Self {
        if err.is_timeout() {
            Self::Timeout { timeout }
        } else if err.is_connect() {
            Self::Connection {
                message: err.to_string(),
            }
        } else if err.is_builder() {
            Self::RequestBuild {
                message: err.to_string(),
            }
        } else if err.is_body() || err.is_decode() {
            Self::ResponseBody {
                message: err.to_string(),
            }
        } else {
            Self::Transport {
                message: err.to_string(),
            }
        }
    }
}

// ============================================================================
// Configuration
// ============================================================================

/// Default request timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Authentication configuration for the HTTP client.
///
/// Determines how the `Authorization` header is constructed for each request.
pub enum ClientAuthConfig {
    /// HTTP Basic authentication using API key credentials.
    Basic {
        /// API credentials (shared via `Arc` to avoid cloning secrets).
        credentials: Arc<Credentials>,
    },
    /// OAuth 2.0 bearer token authentication.
    Bearer {
        /// The OAuth 2.0 access token.
        access_token: AccessToken,
    },
}

/// Configuration for constructing an [`OnshapeClient`].
pub struct ClientConfig {
    /// Base URL for the Onshape API (e.g., `https://cad.onshape.com/api/v14`).
    pub base_url: String,
    /// Authentication configuration.
    pub auth: ClientAuthConfig,
    /// Request timeout. Defaults to 30 seconds if `None`.
    pub timeout: Option<Duration>,
}

// ============================================================================
// Client
// ============================================================================

/// HTTP client for making authenticated requests to the Onshape API.
///
/// Wraps a `reqwest::Client` with Onshape-specific configuration: base URL,
/// authentication, and timeout. The inner `reqwest::Client` uses connection
/// pooling, so cloning `OnshapeClient` is cheap (all clones share the pool).
#[derive(Clone)]
pub struct OnshapeClient {
    http: Client,
    base_url: String,
    auth: ClientAuthConfig,
    timeout: Duration,
}

impl Clone for ClientAuthConfig {
    fn clone(&self) -> Self {
        match self {
            Self::Basic { credentials } => Self::Basic {
                credentials: Arc::clone(credentials),
            },
            Self::Bearer { access_token } => Self::Bearer {
                access_token: access_token.clone(),
            },
        }
    }
}

impl OnshapeClient {
    /// Creates a new client from the given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::RequestBuild`] if the `reqwest::Client` cannot be
    /// constructed (e.g., TLS backend initialization failure).
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        let timeout = config.timeout.unwrap_or(DEFAULT_TIMEOUT);

        let http =
            Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|e| ClientError::RequestBuild {
                    message: format!("failed to build HTTP client: {e}"),
                })?;

        // Strip trailing slash from base URL for consistent joining.
        let base_url = config.base_url.trim_end_matches('/').to_string();

        Ok(Self {
            http,
            base_url,
            auth: config.auth,
            timeout,
        })
    }

    /// Execute an API request against the Onshape server.
    ///
    /// Combines the base URL with the request path, adds authentication headers,
    /// query parameters, and body, then executes the HTTP request.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] for transport-level failures (connection,
    /// timeout, DNS, TLS, etc.). HTTP error status codes (4xx, 5xx) are
    /// returned as successful [`ApiResponse`] values — the caller decides
    /// how to interpret them.
    pub async fn execute(&self, request: &ApiRequest) -> Result<ApiResponse, ClientError> {
        let url = format!("{}{}", self.base_url, request.path);

        let auth_header = match &self.auth {
            ClientAuthConfig::Basic { credentials } => {
                basic_authorization_header_value(credentials)
            }
            ClientAuthConfig::Bearer { access_token } => {
                bearer_authorization_header_value(access_token)
            }
        };

        let method = to_reqwest_method(request.method);

        let mut builder = self
            .http
            .request(method, &url)
            .header("Authorization", auth_header.expose_secret())
            .header("Accept", "application/json");

        // Add query parameters.
        if !request.query_params.is_empty() {
            builder = builder.query(&request.query_params);
        }

        // Add request body.
        match &request.body {
            Some(RequestBody::Json(value)) => {
                // Serialize manually instead of using
                // `reqwest::RequestBuilder::json()` so that the Content-Type
                // header from the OpenAPI spec (e.g.
                // "application/json;charset=UTF-8; qs=0.09") is preserved
                // rather than being overwritten by `.json()`.
                let content_type = request
                    .content_type
                    .as_deref()
                    .unwrap_or("application/json");
                let serialized =
                    serde_json::to_vec(value).map_err(|e| ClientError::ResponseBody {
                        message: format!("failed to serialize request body: {e}"),
                    })?;
                builder = builder
                    .header("Content-Type", content_type)
                    .body(serialized);
            }
            Some(RequestBody::Multipart(multipart)) => {
                // Build a multipart/form-data request.  `reqwest` sets the
                // Content-Type header (with boundary) automatically when
                // `.multipart()` is used — we must NOT set it manually.
                let mut form = reqwest::multipart::Form::new();
                for (name, value) in &multipart.text_fields {
                    form = form.text(name.clone(), value.clone());
                }
                for field in &multipart.binary_fields {
                    let mut part = reqwest::multipart::Part::bytes(field.data.clone());
                    if let Some(ct) = &field.content_type {
                        part = part.mime_str(ct).map_err(|e| ClientError::ResponseBody {
                            message: format!(
                                "invalid MIME type for field `{}`: {e}",
                                field.field_name
                            ),
                        })?;
                    }
                    form = form.part(field.field_name.clone(), part);
                }
                builder = builder.multipart(form);
            }
            None => {}
        }

        let response = builder
            .send()
            .await
            .map_err(|e| ClientError::from_reqwest(&e, self.timeout))?;

        let status = response.status().as_u16();
        let headers = response_headers_to_pairs(response.headers());
        let body = response
            .bytes()
            .await
            .map_err(|e| ClientError::ResponseBody {
                message: e.to_string(),
            })?;

        Ok(ApiResponse {
            status,
            headers,
            body: ResponseBody::from(body.to_vec()),
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Convert our sans-IO `HttpMethod` to a `reqwest::Method`.
const fn to_reqwest_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Patch => reqwest::Method::PATCH,
    }
}

fn response_headers_to_pairs(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use secrecy::SecretString;

    fn test_credentials() -> Arc<Credentials> {
        Arc::new(Credentials {
            access_key: SecretString::from("test_access"),
            secret_key: SecretString::from("test_secret"),
        })
    }

    fn test_config() -> ClientConfig {
        ClientConfig {
            base_url: "https://cad.onshape.com/api/v14".to_string(),
            auth: ClientAuthConfig::Basic {
                credentials: test_credentials(),
            },
            timeout: Some(Duration::from_secs(10)),
        }
    }

    fn test_oauth_config() -> ClientConfig {
        ClientConfig {
            base_url: "https://cad.onshape.com/api/v14".to_string(),
            auth: ClientAuthConfig::Bearer {
                access_token: AccessToken::new("test-oauth-token".to_string()),
            },
            timeout: Some(Duration::from_secs(10)),
        }
    }

    #[test]
    fn client_creation_succeeds_basic() {
        let client = OnshapeClient::new(test_config());
        assert!(client.is_ok());
    }

    #[test]
    fn client_creation_succeeds_oauth() {
        let client = OnshapeClient::new(test_oauth_config());
        assert!(client.is_ok());
    }

    #[test]
    fn client_strips_trailing_slash_from_base_url() {
        let mut config = test_config();
        config.base_url = "https://cad.onshape.com/api/v14/".to_string();
        let client = OnshapeClient::new(config).expect("should create client");
        assert_eq!(client.base_url, "https://cad.onshape.com/api/v14");
    }

    #[test]
    fn client_uses_default_timeout_when_none() {
        let mut config = test_config();
        config.timeout = None;
        let client = OnshapeClient::new(config).expect("should create client");
        assert_eq!(client.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn client_uses_custom_timeout() {
        let mut config = test_config();
        config.timeout = Some(Duration::from_secs(60));
        let client = OnshapeClient::new(config).expect("should create client");
        assert_eq!(client.timeout, Duration::from_secs(60));
    }

    #[test]
    fn client_is_clone() {
        let client = OnshapeClient::new(test_config()).expect("should create client");
        #[allow(clippy::redundant_clone)]
        let _cloned = client.clone();
    }

    #[test]
    fn oauth_client_is_clone() {
        let client = OnshapeClient::new(test_oauth_config()).expect("should create client");
        #[allow(clippy::redundant_clone)]
        let _cloned = client.clone();
    }

    #[test]
    fn to_reqwest_method_maps_correctly() {
        assert_eq!(to_reqwest_method(HttpMethod::Get), reqwest::Method::GET);
        assert_eq!(to_reqwest_method(HttpMethod::Post), reqwest::Method::POST);
        assert_eq!(to_reqwest_method(HttpMethod::Put), reqwest::Method::PUT);
        assert_eq!(
            to_reqwest_method(HttpMethod::Delete),
            reqwest::Method::DELETE
        );
        assert_eq!(to_reqwest_method(HttpMethod::Patch), reqwest::Method::PATCH);
    }

    #[test]
    fn response_headers_to_pairs_preserves_header_values() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Content-Type",
            "application/octet-stream".parse().expect("valid header"),
        );
        headers.insert("X-Test", "value".parse().expect("valid header"));

        let pairs = response_headers_to_pairs(&headers);

        assert!(pairs.contains(&(
            "content-type".to_string(),
            "application/octet-stream".to_string(),
        )));
        assert!(pairs.contains(&("x-test".to_string(), "value".to_string())));
    }

    #[tokio::test]
    async fn execute_returns_binary_body_and_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
        let address = listener
            .local_addr()
            .expect("should get test server address");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("should accept request");
            let mut buffer = [0_u8; 4096];
            let received = stream.read(&mut buffer).expect("should read request");
            let request = String::from_utf8_lossy(&buffer[..received]).to_lowercase();
            assert!(
                request.contains("accept: application/json"),
                "request should preserve JSON Accept header, got: {request}"
            );

            let body = [0_u8, 159, 146, 150];
            let mut response = Vec::new();
            response.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
            response.extend_from_slice(b"Content-Type: application/octet-stream\r\n");
            response.extend_from_slice(b"X-Test: binary\r\n");
            response.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
            response.extend_from_slice(b"Connection: close\r\n\r\n");
            response.extend_from_slice(&body);
            stream.write_all(&response).expect("should write response");
        });

        let client = OnshapeClient::new(ClientConfig {
            base_url: format!("http://{address}"),
            auth: ClientAuthConfig::Basic {
                credentials: test_credentials(),
            },
            timeout: Some(Duration::from_secs(10)),
        })
        .expect("should create client");
        let request = ApiRequest {
            method: HttpMethod::Get,
            path: "/binary".to_string(),
            query_params: Vec::new(),
            body: None,
            content_type: None,
        };

        let response = client
            .execute(&request)
            .await
            .expect("request should succeed");
        server.join().expect("server thread should finish");

        assert_eq!(response.status, 200);
        assert_eq!(response.content_type(), Some("application/octet-stream"));
        assert_eq!(response.header("x-test"), Some("binary"));
        assert_eq!(response.body.as_bytes(), &[0, 159, 146, 150]);
        assert!(response.body.text().is_err());
    }
}
