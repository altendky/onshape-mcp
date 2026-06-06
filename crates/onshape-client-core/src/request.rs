//! HTTP request types for the Onshape API.
//!
//! These types represent API requests as pure data — no I/O is performed here.
//! The I/O layer (`onshape-client-io`) interprets these to make actual HTTP calls.

use std::borrow::Cow;
use std::str::{self, Utf8Error};

use http::{HeaderMap, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// HTTP Method
// ============================================================================

/// Error returned when parsing an unrecognized HTTP method string.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown HTTP method: {0}")]
pub struct UnknownHttpMethod(pub String);

fn parse_method_case_insensitive(s: &str) -> Result<Method, UnknownHttpMethod> {
    Method::from_bytes(s.to_ascii_uppercase().as_bytes())
        .map_err(|_| UnknownHttpMethod(s.to_string()))
}

// `ApiRequest` serde is primarily for test and inspection ergonomics: internal
// tests assert the request shape, and external consumers can serialize requests
// in their own tests/debug tooling. Runtime request execution uses the typed
// fields directly, so these helpers preserve that plain-data surface while the
// boundary uses standard `http` types.
mod method_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Method, parse_method_case_insensitive};

    pub fn serialize<S>(method: &Method, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(method.as_str())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Method, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_method_case_insensitive(&value).map_err(serde::de::Error::custom)
    }
}

// See `method_serde` for why `ApiRequest` keeps serde support around `http`
// protocol types.
mod request_headers_serde {
    use std::str;

    use http::{HeaderMap, HeaderName, HeaderValue};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(headers: &HeaderMap, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let pairs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| {
                value
                    .to_str()
                    .map(|value| (name.as_str(), value))
                    .map_err(serde::ser::Error::custom)
            })
            .collect::<Result<_, _>>()?;
        pairs.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HeaderMap, D::Error>
    where
        D: Deserializer<'de>,
    {
        let pairs = Vec::<(String, String)>::deserialize(deserializer)?;
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(serde::de::Error::custom)?;
            let value = HeaderValue::from_str(&value).map_err(serde::de::Error::custom)?;
            headers.append(name, value);
        }
        Ok(headers)
    }
}

// ============================================================================
// API Request
// ============================================================================

/// An HTTP request to the Onshape API, produced as an effect by `build_request`.
///
/// This is a pure data structure describing what HTTP call to make.
/// It does not include the base URL or authentication — those are added
/// by the I/O layer when executing the request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    /// HTTP method.
    #[serde(with = "method_serde")]
    pub method: Method,
    /// Fully resolved URL path (path params substituted), e.g. `/documents/abc123`.
    pub path: String,
    /// Query parameters.
    pub query_params: Vec<(String, String)>,
    /// Request headers supplied by the request builder.
    #[serde(default, with = "request_headers_serde")]
    pub headers: HeaderMap,
    /// Request body, if any.
    pub body: Option<RequestBody>,
    /// Content type for the request body.
    pub content_type: Option<String>,
}

// ============================================================================
// Request Body
// ============================================================================

/// The body of an API request.
///
/// Different content types require different body representations. The I/O
/// layer uses this to decide how to serialize and send the body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RequestBody {
    /// A JSON body — serialized via `serde_json`.
    Json(Value),
    /// A multipart form body — text fields plus binary file parts.
    /// The I/O layer builds a `multipart/form-data` request from this.
    Multipart(MultipartBody),
}

/// A multipart form body with text and binary parts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MultipartBody {
    /// Text form fields: `(field_name, value)`.
    pub text_fields: Vec<(String, String)>,
    /// Binary form fields (e.g., file uploads).
    pub binary_fields: Vec<BinaryField>,
}

/// A single binary field in a multipart form.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BinaryField {
    /// The form field name (must match the schema property name).
    pub field_name: String,
    /// The raw binary content.
    pub data: Vec<u8>,
    /// Optional MIME type for this part (e.g., `application/octet-stream`).
    pub content_type: Option<String>,
}

impl RequestBody {
    /// Convenience: extract the inner [`Value`] if this is a `Json` variant.
    ///
    /// Returns `None` for non-JSON variants.
    #[must_use]
    pub const fn as_json(&self) -> Option<&Value> {
        match self {
            Self::Json(v) => Some(v),
            Self::Multipart(_) => None,
        }
    }
}

// ============================================================================
// API Response
// ============================================================================

/// A raw HTTP response from the Onshape API.
///
/// This is the minimal data the I/O layer returns after executing an [`ApiRequest`].
/// Higher layers interpret the status code and body as needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiResponse {
    /// HTTP status code.
    pub status: StatusCode,
    /// Response headers.
    pub headers: HeaderMap,
    /// Response body bytes.
    pub body: ResponseBody,
}

impl ApiResponse {
    /// Return the first response header value matching `name`, case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Return the response `Content-Type` header, if present.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }
}

/// A buffered response body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseBody {
    /// Raw response body bytes.
    pub bytes: Vec<u8>,
}

impl ResponseBody {
    /// Create a buffered response body from raw bytes.
    #[must_use]
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Return the response body as raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Decode the response body as strict UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns an error if the response contains invalid UTF-8.
    pub fn text(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(&self.bytes)
    }

    /// Decode the response body as text, replacing invalid UTF-8 sequences.
    #[must_use]
    pub fn text_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }
}

impl From<Vec<u8>> for ResponseBody {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<String> for ResponseBody {
    fn from(text: String) -> Self {
        Self::new(text.into_bytes())
    }
}

impl From<&str> for ResponseBody {
    fn from(text: &str) -> Self {
        Self::new(text.as_bytes().to_vec())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn http_method_from_str_lowercase() {
        assert_eq!(parse_method_case_insensitive("get"), Ok(Method::GET));
        assert_eq!(parse_method_case_insensitive("post"), Ok(Method::POST));
        assert_eq!(parse_method_case_insensitive("put"), Ok(Method::PUT));
        assert_eq!(parse_method_case_insensitive("delete"), Ok(Method::DELETE));
        assert_eq!(parse_method_case_insensitive("patch"), Ok(Method::PATCH));
    }

    #[test]
    fn http_method_from_str_uppercase() {
        assert_eq!(parse_method_case_insensitive("GET"), Ok(Method::GET));
        assert_eq!(parse_method_case_insensitive("POST"), Ok(Method::POST));
    }

    #[test]
    fn http_method_from_str_mixed_case() {
        assert_eq!(parse_method_case_insensitive("Get"), Ok(Method::GET));
        assert_eq!(parse_method_case_insensitive("PoSt"), Ok(Method::POST));
    }

    #[test]
    fn http_method_from_str_unknown() {
        assert!(parse_method_case_insensitive("").is_err());
    }

    #[test]
    fn http_method_display() {
        assert_eq!(Method::GET.to_string(), "GET");
        assert_eq!(Method::POST.to_string(), "POST");
        assert_eq!(Method::PUT.to_string(), "PUT");
        assert_eq!(Method::DELETE.to_string(), "DELETE");
        assert_eq!(Method::PATCH.to_string(), "PATCH");
    }

    #[test]
    fn api_request_serializes() {
        let req = ApiRequest {
            method: Method::GET,
            path: "/documents/abc123".to_string(),
            query_params: vec![("limit".to_string(), "10".to_string())],
            headers: HeaderMap::new(),
            body: None,
            content_type: None,
        };
        let json = serde_json::to_value(&req).expect("should serialize");
        assert_eq!(json["method"], "GET");
        assert_eq!(json["path"], "/documents/abc123");
    }

    #[test]
    fn api_request_with_json_body_serializes() {
        let req = ApiRequest {
            method: Method::POST,
            path: "/documents".to_string(),
            query_params: vec![],
            headers: HeaderMap::new(),
            body: Some(RequestBody::Json(serde_json::json!({"name": "test"}))),
            content_type: Some("application/json".to_string()),
        };
        let json = serde_json::to_value(&req).expect("should serialize");
        assert_eq!(json["method"], "POST");
        assert!(json["body"].is_object());
    }

    #[test]
    fn request_body_as_json() {
        let json_body = RequestBody::Json(serde_json::json!({"key": "value"}));
        assert!(json_body.as_json().is_some());

        let multipart_body = RequestBody::Multipart(MultipartBody {
            text_fields: vec![],
            binary_fields: vec![],
        });
        assert!(multipart_body.as_json().is_none());
    }

    #[test]
    fn response_body_text_decodes_strict_utf8() {
        let body = ResponseBody::from("plain text");
        assert_eq!(body.as_bytes(), b"plain text");
        assert_eq!(body.text().expect("should decode"), "plain text");
    }

    #[test]
    fn response_body_text_rejects_invalid_utf8() {
        let body = ResponseBody::from(vec![0xff]);
        assert!(body.text().is_err());
        assert_eq!(body.text_lossy().as_ref(), "\u{fffd}");
    }

    #[test]
    fn api_response_header_lookup_is_case_insensitive() {
        let response = ApiResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            )]),
            body: ResponseBody::from("{}"),
        };

        assert_eq!(response.header("content-type"), Some("application/json"));
        assert_eq!(response.header("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(response.content_type(), Some("application/json"));
        assert_eq!(response.header("x-missing"), None);
    }
}
