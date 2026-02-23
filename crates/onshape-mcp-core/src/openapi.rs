//! `OpenAPI` spec parsing, searching, and request building.
//!
//! This module provides pure (sans-IO) operations over an Onshape `OpenAPI` specification.
//! The spec JSON content is provided externally; this module never performs I/O.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur when working with the `OpenAPI` spec.
#[derive(Debug, thiserror::Error)]
pub enum OpenApiError {
    /// Failed to parse the `OpenAPI` JSON.
    #[error("failed to parse OpenAPI spec: {0}")]
    ParseError(#[from] serde_json::Error),

    /// The spec is missing required fields.
    #[error("invalid OpenAPI spec: {reason}")]
    InvalidSpec { reason: String },

    /// The requested endpoint was not found.
    #[error("endpoint not found: {endpoint_id}")]
    EndpointNotFound { endpoint_id: String },

    /// Invalid parameters for an API call.
    #[error("invalid parameters: {reason}")]
    InvalidParams { reason: String },
}

// ============================================================================
// Public Types
// ============================================================================

/// HTTP method for an API request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "get" => Some(Self::Get),
            "post" => Some(Self::Post),
            "put" => Some(Self::Put),
            "delete" => Some(Self::Delete),
            "patch" => Some(Self::Patch),
            _ => None,
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

/// Brief summary of an endpoint, returned by search.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EndpointSummary {
    /// The operation ID (unique identifier for this endpoint).
    pub operation_id: String,
    /// HTTP method.
    pub method: HttpMethod,
    /// URL path template (e.g., `/documents/{did}`).
    pub path: String,
    /// One-line description of the endpoint.
    pub description: String,
    /// Tags associated with this endpoint.
    pub tags: Vec<String>,
}

/// Parameter location in the HTTP request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    Path,
    Query,
    Header,
}

/// Description of a single parameter.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ParameterDetail {
    /// Parameter name.
    pub name: String,
    /// Where the parameter appears (path, query, header).
    pub location: ParameterLocation,
    /// Whether the parameter is required.
    pub required: bool,
    /// Parameter type (e.g., "string", "integer").
    pub param_type: String,
    /// Description of the parameter.
    pub description: String,
    /// Default value, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Enum values, if constrained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<Value>>,
}

/// Full detail of an endpoint, returned by explain.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EndpointDetail {
    /// The operation ID.
    pub operation_id: String,
    /// HTTP method.
    pub method: HttpMethod,
    /// URL path template.
    pub path: String,
    /// Full description of the endpoint.
    pub description: String,
    /// Tags associated with this endpoint.
    pub tags: Vec<String>,
    /// Parameters (path, query, header).
    pub parameters: Vec<ParameterDetail>,
    /// Whether the endpoint accepts a request body.
    pub has_request_body: bool,
    /// Request body schema (JSON Schema), if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body_schema: Option<Value>,
    /// Content type for the request body, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body_content_type: Option<String>,
    /// Response schema (JSON Schema) for the success response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
}

/// An HTTP request to the Onshape API, produced as an effect by `build_request`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    /// HTTP method.
    pub method: HttpMethod,
    /// Fully resolved URL path (path params substituted).
    pub path: String,
    /// Query parameters.
    pub query_params: Vec<(String, String)>,
    /// Request body, if any.
    pub body: Option<Value>,
    /// Content type for the request body.
    pub content_type: Option<String>,
}

/// Filters for searching endpoints.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct SearchFilters {
    /// Filter by HTTP method (e.g., "GET", "POST").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Filter by tag name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

// ============================================================================
// Internal Types
// ============================================================================

/// A parsed endpoint from the `OpenAPI` spec.
#[derive(Clone, Debug)]
struct ParsedEndpoint {
    operation_id: String,
    method: HttpMethod,
    path: String,
    summary: String,
    description: String,
    tags: Vec<String>,
    parameters: Vec<ParsedParameter>,
    has_request_body: bool,
    request_body_schema: Option<Value>,
    request_body_content_type: Option<String>,
    response_schema: Option<Value>,
    /// Lowercased text for search matching (operationId + path + summary + description + tags).
    search_text: String,
}

#[derive(Clone, Debug)]
struct ParsedParameter {
    name: String,
    location: ParameterLocation,
    required: bool,
    param_type: String,
    description: String,
    default: Option<Value>,
    enum_values: Option<Vec<Value>>,
}

// ============================================================================
// OpenApiSpec
// ============================================================================

/// A parsed and indexed `OpenAPI` specification.
#[derive(Debug)]
pub struct OpenApiSpec {
    /// The base server URL (e.g., `https://cad.onshape.com/api/v14`).
    server_url: String,
    /// All endpoints, indexed by operation ID.
    endpoints: HashMap<String, ParsedEndpoint>,
    /// Ordered list of operation IDs (for consistent search output).
    operation_ids: Vec<String>,
}

impl OpenApiSpec {
    /// Parse an `OpenAPI` specification from a JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is malformed or missing required fields.
    pub fn from_json(json: &str) -> Result<Self, OpenApiError> {
        let root: Value = serde_json::from_str(json)?;
        Self::from_value(&root)
    }

    /// Parse an `OpenAPI` specification from a `serde_json::Value`.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is missing required fields.
    pub fn from_value(root: &Value) -> Result<Self, OpenApiError> {
        // Extract server URL
        let server_url = root
            .pointer("/servers/0/url")
            .and_then(Value::as_str)
            .unwrap_or("https://cad.onshape.com/api/v6")
            .to_string();

        // Extract component schemas for $ref resolution
        let components = Self::extract_components(root);

        // Parse all endpoints
        let paths = root
            .get("paths")
            .and_then(Value::as_object)
            .ok_or_else(|| OpenApiError::InvalidSpec {
                reason: "missing 'paths' object".into(),
            })?;

        let mut endpoints = HashMap::new();
        let mut operation_ids = Vec::new();

        for (path, methods_val) in paths {
            let Some(methods) = methods_val.as_object() else {
                continue;
            };
            for (method_str, detail) in methods {
                let Some(method) = HttpMethod::from_str(method_str) else {
                    continue;
                };
                let Some(operation_id) = detail.get("operationId").and_then(Value::as_str) else {
                    continue;
                };

                let endpoint =
                    Self::parse_endpoint(operation_id, method, path, detail, &components);
                operation_ids.push(operation_id.to_string());
                endpoints.insert(operation_id.to_string(), endpoint);
            }
        }

        Ok(Self {
            server_url,
            endpoints,
            operation_ids,
        })
    }

    /// Returns the base server URL.
    #[must_use]
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Returns the number of endpoints in the spec.
    #[must_use]
    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    /// Search for endpoints matching a query string and optional filters.
    ///
    /// Performs case-insensitive substring matching against operation ID, path,
    /// summary, description, and tags.
    #[must_use]
    pub fn search(&self, query: &str, filters: &SearchFilters) -> Vec<EndpointSummary> {
        let query_lower = query.to_lowercase();

        let method_filter = filters.method.as_deref().and_then(HttpMethod::from_str);

        let tag_filter = filters.tag.as_deref().map(str::to_lowercase);

        let mut results = Vec::new();

        for op_id in &self.operation_ids {
            let Some(ep) = self.endpoints.get(op_id) else {
                continue;
            };

            // Apply method filter
            if let Some(ref mf) = method_filter
                && ep.method != *mf
            {
                continue;
            }

            // Apply tag filter
            if let Some(ref tf) = tag_filter {
                let has_tag = ep.tags.iter().any(|t| t.to_lowercase() == *tf);
                if !has_tag {
                    continue;
                }
            }

            // Apply text search
            if !query_lower.is_empty() && !ep.search_text.contains(&query_lower) {
                continue;
            }

            results.push(EndpointSummary {
                operation_id: ep.operation_id.clone(),
                method: ep.method,
                path: ep.path.clone(),
                description: if ep.summary.is_empty() {
                    Self::truncate_description(&ep.description, 120)
                } else {
                    ep.summary.clone()
                },
                tags: ep.tags.clone(),
            });
        }

        results
    }

    /// Get full details for a specific endpoint by operation ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the endpoint is not found.
    pub fn explain(&self, endpoint_id: &str) -> Result<EndpointDetail, OpenApiError> {
        let ep = self
            .endpoints
            .get(endpoint_id)
            .ok_or_else(|| OpenApiError::EndpointNotFound {
                endpoint_id: endpoint_id.to_string(),
            })?;

        Ok(EndpointDetail {
            operation_id: ep.operation_id.clone(),
            method: ep.method,
            path: ep.path.clone(),
            description: if ep.description.is_empty() {
                ep.summary.clone()
            } else {
                ep.description.clone()
            },
            tags: ep.tags.clone(),
            parameters: ep
                .parameters
                .iter()
                .map(|p| ParameterDetail {
                    name: p.name.clone(),
                    location: p.location,
                    required: p.required,
                    param_type: p.param_type.clone(),
                    description: p.description.clone(),
                    default: p.default.clone(),
                    enum_values: p.enum_values.clone(),
                })
                .collect(),
            has_request_body: ep.has_request_body,
            request_body_schema: ep.request_body_schema.clone(),
            request_body_content_type: ep.request_body_content_type.clone(),
            response_schema: ep.response_schema.clone(),
        })
    }

    /// Build an API request effect for a given endpoint.
    ///
    /// Validates that required path parameters are provided and substitutes them
    /// into the path template. Query parameters and body are passed through.
    ///
    /// # Errors
    ///
    /// Returns an error if the endpoint is not found or required parameters are missing.
    pub fn build_request(
        &self,
        endpoint_id: &str,
        path_params: &HashMap<String, String>,
        query_params: &HashMap<String, String>,
        body: Option<Value>,
    ) -> Result<ApiRequest, OpenApiError> {
        let ep = self
            .endpoints
            .get(endpoint_id)
            .ok_or_else(|| OpenApiError::EndpointNotFound {
                endpoint_id: endpoint_id.to_string(),
            })?;

        // Validate required path parameters
        let mut resolved_path = ep.path.clone();
        for param in &ep.parameters {
            if param.location == ParameterLocation::Path && param.required {
                let value =
                    path_params
                        .get(&param.name)
                        .ok_or_else(|| OpenApiError::InvalidParams {
                            reason: format!("missing required path parameter: {}", param.name),
                        })?;
                resolved_path = resolved_path.replace(&format!("{{{}}}", param.name), value);
            }
        }

        // Also substitute any optional path params that were provided
        for (name, value) in path_params {
            resolved_path = resolved_path.replace(&format!("{{{name}}}"), value);
        }

        // Validate required query parameters
        for param in &ep.parameters {
            if param.location == ParameterLocation::Query
                && param.required
                && !query_params.contains_key(&param.name)
            {
                return Err(OpenApiError::InvalidParams {
                    reason: format!("missing required query parameter: {}", param.name),
                });
            }
        }

        // Validate request body
        if ep.has_request_body && body.is_none() {
            // Some request bodies are optional, so we just note it but don't error
        }

        let query_params_vec: Vec<(String, String)> = query_params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Ok(ApiRequest {
            method: ep.method,
            path: resolved_path,
            query_params: query_params_vec,
            body,
            content_type: ep.request_body_content_type.clone(),
        })
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    fn extract_components(root: &Value) -> HashMap<String, Value> {
        let mut components = HashMap::new();
        if let Some(schemas) = root
            .pointer("/components/schemas")
            .and_then(Value::as_object)
        {
            for (name, schema) in schemas {
                components.insert(name.clone(), schema.clone());
            }
        }
        components
    }

    fn parse_endpoint(
        operation_id: &str,
        method: HttpMethod,
        path: &str,
        detail: &Value,
        components: &HashMap<String, Value>,
    ) -> ParsedEndpoint {
        let summary = detail
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let description = detail
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let tags: Vec<String> = detail
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        let parameters = Self::parse_parameters(detail);
        let (has_request_body, request_body_schema, request_body_content_type) =
            Self::parse_request_body(detail, components);
        let response_schema = Self::parse_response_schema(detail, components);

        // Build search text
        let search_text = format!(
            "{} {} {} {} {}",
            operation_id.to_lowercase(),
            path.to_lowercase(),
            summary.to_lowercase(),
            description.to_lowercase(),
            tags.iter()
                .map(|t| t.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ")
        );

        ParsedEndpoint {
            operation_id: operation_id.to_string(),
            method,
            path: path.to_string(),
            summary,
            description,
            tags,
            parameters,
            has_request_body,
            request_body_schema,
            request_body_content_type,
            response_schema,
            search_text,
        }
    }

    fn parse_parameters(detail: &Value) -> Vec<ParsedParameter> {
        let Some(params) = detail.get("parameters").and_then(Value::as_array) else {
            return Vec::new();
        };

        params
            .iter()
            .filter_map(|p| {
                let name = p.get("name").and_then(Value::as_str)?.to_string();
                let location = match p.get("in").and_then(Value::as_str)? {
                    "path" => ParameterLocation::Path,
                    "query" => ParameterLocation::Query,
                    "header" => ParameterLocation::Header,
                    _ => return None,
                };
                let required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
                let schema = p.get("schema");
                let param_type = schema
                    .and_then(|s| s.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("string")
                    .to_string();
                let description = p
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let default = schema.and_then(|s| s.get("default")).cloned();
                let enum_values = schema
                    .and_then(|s| s.get("enum"))
                    .and_then(Value::as_array)
                    .cloned();

                Some(ParsedParameter {
                    name,
                    location,
                    required,
                    param_type,
                    description,
                    default,
                    enum_values,
                })
            })
            .collect()
    }

    fn parse_request_body(
        detail: &Value,
        components: &HashMap<String, Value>,
    ) -> (bool, Option<Value>, Option<String>) {
        let Some(rb) = detail.get("requestBody") else {
            return (false, None, None);
        };

        let content = rb.get("content").and_then(Value::as_object);
        if let Some(content_map) = content {
            // Pick the first content type (usually application/json)
            if let Some((content_type, schema_info)) = content_map.iter().next() {
                let schema = schema_info.get("schema").cloned();
                let resolved = schema.map(|s| Self::resolve_ref_shallow(&s, components));
                return (true, resolved, Some(content_type.clone()));
            }
        }

        (true, None, None)
    }

    fn parse_response_schema(detail: &Value, components: &HashMap<String, Value>) -> Option<Value> {
        let responses = detail.get("responses")?.as_object()?;

        // Look for 200 or 2xx response
        let response = responses
            .get("200")
            .or_else(|| responses.get("201"))
            .or_else(|| responses.get("2XX"))?;

        let content = response.get("content")?.as_object()?;
        let (_, schema_info) = content.iter().next()?;
        let schema = schema_info.get("schema")?;

        Some(Self::resolve_ref_shallow(schema, components))
    }

    /// Resolve a single level of `$ref` — replaces the `$ref` pointer with the
    /// referenced schema. Does NOT recursively resolve nested `$ref`s (to avoid
    /// unbounded expansion of the spec).
    fn resolve_ref_shallow(schema: &Value, components: &HashMap<String, Value>) -> Value {
        if let Some(ref_str) = schema.get("$ref").and_then(Value::as_str)
            && let Some(name) = ref_str.strip_prefix("#/components/schemas/")
            && let Some(resolved) = components.get(name)
        {
            return resolved.clone();
        }
        schema.clone()
    }

    fn truncate_description(s: &str, max_len: usize) -> String {
        if s.len() <= max_len {
            s.to_string()
        } else {
            format!("{}...", &s[..max_len - 3])
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A minimal `OpenAPI` spec for testing.
    #[allow(clippy::too_many_lines)]
    fn test_spec_json() -> &'static str {
        r##"{
            "openapi": "3.0.1",
            "info": { "title": "Test API", "version": "1.0" },
            "servers": [{ "url": "https://example.com/api/v1" }],
            "paths": {
                "/documents": {
                    "get": {
                        "operationId": "getDocuments",
                        "summary": "List user documents",
                        "description": "Get a list of documents.",
                        "tags": ["Document"],
                        "parameters": [
                            {
                                "name": "q",
                                "in": "query",
                                "required": false,
                                "schema": { "type": "string" },
                                "description": "Search query"
                            },
                            {
                                "name": "limit",
                                "in": "query",
                                "required": false,
                                "schema": { "type": "integer", "default": 20 },
                                "description": "Max results"
                            }
                        ],
                        "responses": {
                            "200": {
                                "content": {
                                    "application/json": {
                                        "schema": { "$ref": "#/components/schemas/DocumentList" }
                                    }
                                }
                            }
                        }
                    },
                    "post": {
                        "operationId": "createDocument",
                        "summary": "Create a document",
                        "tags": ["Document"],
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/CreateDocParams" }
                                }
                            }
                        },
                        "responses": {
                            "200": {
                                "content": {
                                    "application/json": {
                                        "schema": { "$ref": "#/components/schemas/DocumentInfo" }
                                    }
                                }
                            }
                        }
                    }
                },
                "/documents/{did}": {
                    "get": {
                        "operationId": "getDocument",
                        "summary": "Get document by ID",
                        "tags": ["Document"],
                        "parameters": [
                            {
                                "name": "did",
                                "in": "path",
                                "required": true,
                                "schema": { "type": "string" },
                                "description": "Document ID"
                            }
                        ],
                        "responses": {
                            "200": {
                                "content": {
                                    "application/json": {
                                        "schema": { "$ref": "#/components/schemas/DocumentInfo" }
                                    }
                                }
                            }
                        }
                    },
                    "delete": {
                        "operationId": "deleteDocument",
                        "summary": "Delete a document",
                        "tags": ["Document"],
                        "parameters": [
                            {
                                "name": "did",
                                "in": "path",
                                "required": true,
                                "schema": { "type": "string" },
                                "description": "Document ID"
                            }
                        ],
                        "responses": { "200": {} }
                    }
                },
                "/parts": {
                    "get": {
                        "operationId": "listParts",
                        "summary": "List parts in a studio",
                        "tags": ["PartStudio"],
                        "parameters": [],
                        "responses": { "200": {} }
                    }
                }
            },
            "components": {
                "schemas": {
                    "DocumentList": {
                        "type": "object",
                        "properties": {
                            "items": { "type": "array" }
                        }
                    },
                    "DocumentInfo": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" }
                        }
                    },
                    "CreateDocParams": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        },
                        "required": ["name"]
                    }
                }
            }
        }"##
    }

    #[test]
    fn parse_spec() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        assert_eq!(spec.endpoint_count(), 5);
        assert_eq!(spec.server_url(), "https://example.com/api/v1");
    }

    #[test]
    fn search_by_keyword() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let results = spec.search("document", &SearchFilters::default());
        // Should match getDocuments, createDocument, getDocument, deleteDocument
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn search_empty_query_returns_all() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let results = spec.search("", &SearchFilters::default());
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn search_filter_by_method() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let results = spec.search(
            "",
            &SearchFilters {
                method: Some("DELETE".to_string()),
                ..SearchFilters::default()
            },
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].operation_id, "deleteDocument");
    }

    #[test]
    fn search_filter_by_tag() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let results = spec.search(
            "",
            &SearchFilters {
                tag: Some("PartStudio".to_string()),
                ..SearchFilters::default()
            },
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].operation_id, "listParts");
    }

    #[test]
    fn search_combined_query_and_filter() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let results = spec.search(
            "document",
            &SearchFilters {
                method: Some("GET".to_string()),
                ..SearchFilters::default()
            },
        );
        // Only GET operations that match "document"
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.method == HttpMethod::Get));
    }

    #[test]
    fn explain_existing_endpoint() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec.explain("getDocuments").expect("should find");

        assert_eq!(detail.operation_id, "getDocuments");
        assert_eq!(detail.method, HttpMethod::Get);
        assert_eq!(detail.path, "/documents");
        assert_eq!(detail.parameters.len(), 2);
        assert!(!detail.has_request_body);
        assert!(detail.response_schema.is_some());
    }

    #[test]
    fn explain_endpoint_with_request_body() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec.explain("createDocument").expect("should find");

        assert!(detail.has_request_body);
        assert!(detail.request_body_schema.is_some());
        assert!(
            detail
                .request_body_content_type
                .as_deref()
                .is_some_and(|ct| ct.contains("application/json"))
        );
    }

    #[test]
    fn explain_nonexistent_endpoint() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let err = spec.explain("nonExistentEndpoint").unwrap_err();
        assert!(matches!(err, OpenApiError::EndpointNotFound { .. }));
    }

    #[test]
    fn build_request_with_path_params() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "abc123".to_string());

        let request = spec
            .build_request("getDocument", &path_params, &HashMap::new(), None)
            .expect("should build");

        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/documents/abc123");
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
    }

    #[test]
    fn build_request_missing_required_path_param() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let err = spec
            .build_request("getDocument", &HashMap::new(), &HashMap::new(), None)
            .unwrap_err();
        assert!(matches!(err, OpenApiError::InvalidParams { .. }));
    }

    #[test]
    fn build_request_with_query_params() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let mut query_params = HashMap::new();
        query_params.insert("q".to_string(), "robot".to_string());
        query_params.insert("limit".to_string(), "10".to_string());

        let request = spec
            .build_request("getDocuments", &HashMap::new(), &query_params, None)
            .expect("should build");

        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/documents");
        assert_eq!(request.query_params.len(), 2);
    }

    #[test]
    fn build_request_with_body() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let body = serde_json::json!({"name": "My Document"});

        let request = spec
            .build_request(
                "createDocument",
                &HashMap::new(),
                &HashMap::new(),
                Some(body),
            )
            .expect("should build");

        assert_eq!(request.method, HttpMethod::Post);
        assert!(request.body.is_some());
        assert!(request.content_type.is_some());
    }

    #[test]
    fn invalid_json_returns_error() {
        let err = OpenApiSpec::from_json("not json").unwrap_err();
        assert!(matches!(err, OpenApiError::ParseError(_)));
    }

    #[test]
    fn missing_paths_returns_error() {
        let err = OpenApiSpec::from_json(r#"{"openapi": "3.0.1"}"#).unwrap_err();
        assert!(matches!(err, OpenApiError::InvalidSpec { .. }));
    }

    #[test]
    fn ref_resolution_works() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec.explain("getDocuments").expect("should find");

        // Response schema should be resolved from $ref
        let schema = detail.response_schema.expect("should have response schema");
        assert!(schema.get("properties").is_some());
    }

    #[test]
    fn parameter_details_include_defaults() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec.explain("getDocuments").expect("should find");

        let limit_param = detail
            .parameters
            .iter()
            .find(|p| p.name == "limit")
            .expect("should have limit param");
        assert_eq!(limit_param.default, Some(Value::from(20)));
    }

    #[test]
    fn search_is_case_insensitive() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let results_lower = spec.search("document", &SearchFilters::default());
        let results_upper = spec.search("DOCUMENT", &SearchFilters::default());
        assert_eq!(results_lower.len(), results_upper.len());
    }
}
