//! HTTP request types for the Onshape API.
//!
//! These types represent API requests as pure data — no I/O is performed here.
//! The I/O layer (`onshape-client-io`) interprets these to make actual HTTP calls.

use std::str::FromStr;

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
    pub body: Option<Value>,
    /// Content type for the request body.
    pub content_type: Option<String>,
}

// ============================================================================
// API Response
// ============================================================================

/// A raw HTTP response from the Onshape API.
///
/// This is the minimal data the I/O layer returns after executing an [`ApiRequest`].
/// Higher layers interpret the status code and body as needed.
#[derive(Clone, Debug)]
pub struct ApiResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body as a string.
    pub body: String,
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
}
