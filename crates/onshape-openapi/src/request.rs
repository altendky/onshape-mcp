//! API-neutral request data for `OpenAPI` request building and MCP effects.
//!
//! Requests contain relative paths, headers, query parameters, and body data.
//! The application supplies the base URL, authentication, and I/O executor.

use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// `ApiRequest` serde is primarily for test and inspection ergonomics: internal
// tests assert the request shape, and external consumers can serialize requests
// in their own tests/debug tooling. Runtime request execution uses the typed
// fields directly, so these helpers preserve that plain-data surface while the
// boundary uses standard `http` types.
mod method_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::Method;

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
        Method::from_bytes(value.to_ascii_uppercase().as_bytes())
            .map_err(|_| serde::de::Error::custom(format!("unknown HTTP method: {value}")))
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

/// An HTTP request produced by [`OpenApiSpec::build_request`](crate::OpenApiSpec::build_request).
///
/// This is a pure data structure describing what HTTP call to make.
/// It does not include the base URL or authentication — those are added
/// by the I/O layer when executing the request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    /// HTTP method.
    #[serde(with = "method_serde")]
    pub method: Method,
    /// Fully resolved URL path (path params substituted), e.g. `/items/abc123`.
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use serde_json::json;

    #[test]
    fn request_round_trip_preserves_body_shapes_and_repeated_parameters() {
        for (body, content_type) in [
            (Value::Null, Value::Null),
            (
                json!({ "Json": { "title": "Example", "optional": null } }),
                json!("application/json"),
            ),
            (
                json!({ "Multipart": {
                    "text_fields": [["tag", "first"], ["tag", "second"]],
                    "binary_fields": [{
                        "field_name": "file",
                        "data": [0, 128, 255],
                        "content_type": "application/octet-stream"
                    }]
                } }),
                json!("multipart/form-data"),
            ),
        ] {
            let encoded = json!({
                "method": "POST",
                "path": "/items/a%2Fb",
                "query_params": [["tag", "first"], ["tag", "second"]],
                "headers": [["x-tag", "first"], ["x-tag", "second"]],
                "body": body,
                "content_type": content_type
            });
            let request: ApiRequest =
                serde_json::from_value(encoded.clone()).expect("valid request");
            assert_eq!(
                serde_json::to_value(&request).expect("serializable request"),
                encoded
            );
            assert_eq!(
                request.body.as_ref().and_then(RequestBody::as_json),
                body.get("Json")
            );
        }
    }

    #[test]
    fn request_deserialization_defaults_headers_and_normalizes_methods() {
        for (input, expected) in [("pAtCh", "PATCH"), ("custom-verb", "CUSTOM-VERB")] {
            let request: ApiRequest = serde_json::from_value(json!({
                "method": input,
                "path": "/items",
                "query_params": []
            }))
            .expect("request without headers or body should parse");
            assert_eq!(request.method.as_str(), expected);
            assert!(request.headers.is_empty());
            assert!(request.body.is_none());
            assert!(request.content_type.is_none());
        }
    }

    #[test]
    fn request_deserialization_rejects_invalid_methods_and_headers() {
        for (field, value) in [
            ("method", json!("bad method")),
            ("headers", json!([["bad header", "value"]])),
            ("headers", json!([["x-test", "bad\nvalue"]])),
        ] {
            let mut encoded = json!({ "method": "GET", "path": "/items", "query_params": [] });
            encoded[field] = value;
            assert!(serde_json::from_value::<ApiRequest>(encoded).is_err());
        }
    }

    #[test]
    fn request_serialization_rejects_non_text_header_values() {
        let request = ApiRequest {
            method: Method::GET,
            path: "/items".into(),
            query_params: vec![],
            headers: HeaderMap::from_iter([(
                http::HeaderName::from_static("x-opaque"),
                HeaderValue::from_bytes(&[0x80]).expect("valid opaque header"),
            )]),
            body: None,
            content_type: None,
        };
        assert!(serde_json::to_value(&request).is_err());
    }
}
