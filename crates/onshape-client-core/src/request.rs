//! HTTP request types for the Onshape API.
//!
//! These types represent API requests as pure data — no I/O is performed here.
//! The I/O layer (`onshape-client-io`) interprets these to make actual HTTP calls.

use std::borrow::Cow;
use std::str::{self, FromStr, Utf8Error};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// HTTP Method
// ============================================================================

/// HTTP method for an API request.
///
/// JSON serialization/deserialization is **uppercase-only** (via `serde(rename_all = "UPPERCASE")`),
/// matching the HTTP convention for method tokens in structured payloads.
/// [`FromStr`] is **case-insensitive** (e.g. `"get"`, `"Get"`, `"GET"` all parse successfully),
/// to accommodate sources like `OpenAPI` spec keys that use lowercase by convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

/// Error returned when parsing an unrecognized HTTP method string.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown HTTP method: {0}")]
pub struct UnknownHttpMethod(pub String);

impl FromStr for HttpMethod {
    type Err = UnknownHttpMethod;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "get" => Ok(Self::Get),
            "post" => Ok(Self::Post),
            "put" => Ok(Self::Put),
            "delete" => Ok(Self::Delete),
            "patch" => Ok(Self::Patch),
            _ => Err(UnknownHttpMethod(s.to_string())),
        }
    }
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
            Self::Put => write!(f, "PUT"),
            Self::Delete => write!(f, "DELETE"),
            Self::Patch => write!(f, "PATCH"),
        }
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
    pub method: HttpMethod,
    /// Fully resolved URL path (path params substituted), e.g. `/documents/abc123`.
    pub path: String,
    /// Query parameters.
    pub query_params: Vec<(String, String)>,
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
    pub status: u16,
    /// Response headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: ResponseBody,
}

impl ApiResponse {
    /// Return the first response header value matching `name`, case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
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
        assert_eq!(HttpMethod::from_str("get"), Ok(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str("post"), Ok(HttpMethod::Post));
        assert_eq!(HttpMethod::from_str("put"), Ok(HttpMethod::Put));
        assert_eq!(HttpMethod::from_str("delete"), Ok(HttpMethod::Delete));
        assert_eq!(HttpMethod::from_str("patch"), Ok(HttpMethod::Patch));
    }

    #[test]
    fn http_method_from_str_uppercase() {
        assert_eq!(HttpMethod::from_str("GET"), Ok(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str("POST"), Ok(HttpMethod::Post));
    }

    #[test]
    fn http_method_from_str_mixed_case() {
        assert_eq!(HttpMethod::from_str("Get"), Ok(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str("PoSt"), Ok(HttpMethod::Post));
    }

    #[test]
    fn http_method_from_str_unknown() {
        assert!(HttpMethod::from_str("TRACE").is_err());
        assert!(HttpMethod::from_str("").is_err());
    }

    #[test]
    fn http_method_display() {
        assert_eq!(HttpMethod::Get.to_string(), "GET");
        assert_eq!(HttpMethod::Post.to_string(), "POST");
        assert_eq!(HttpMethod::Put.to_string(), "PUT");
        assert_eq!(HttpMethod::Delete.to_string(), "DELETE");
        assert_eq!(HttpMethod::Patch.to_string(), "PATCH");
    }

    #[test]
    fn http_method_serializes_uppercase() {
        let json = serde_json::to_string(&HttpMethod::Get).expect("should serialize");
        assert_eq!(json, "\"GET\"");
    }

    #[test]
    fn http_method_deserializes_uppercase() {
        let method: HttpMethod = serde_json::from_str("\"POST\"").expect("should deserialize");
        assert_eq!(method, HttpMethod::Post);
    }

    #[test]
    fn api_request_serializes() {
        let req = ApiRequest {
            method: HttpMethod::Get,
            path: "/documents/abc123".to_string(),
            query_params: vec![("limit".to_string(), "10".to_string())],
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
            method: HttpMethod::Post,
            path: "/documents".to_string(),
            query_params: vec![],
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
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: ResponseBody::from("{}"),
        };

        assert_eq!(response.header("content-type"), Some("application/json"));
        assert_eq!(response.header("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(response.content_type(), Some("application/json"));
        assert_eq!(response.header("x-missing"), None);
    }
}
