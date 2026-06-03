//! `OpenAPI` spec parsing, searching, and request building.
//!
//! This crate provides pure (sans-IO) operations over an Onshape `OpenAPI` specification.
//! The spec JSON content is provided externally; this crate never performs I/O.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use onshape_client_core::request::{ApiRequest, HttpMethod};
use onshape_client_core::request::{BinaryField, MultipartBody, RequestBody};

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

    /// The requested schema was not found.
    #[error("schema not found: {schema_name}")]
    SchemaNotFound { schema_name: String },
}

// ============================================================================
// Public Types
// ============================================================================

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

/// Detail of a component schema, returned by schema lookup.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SchemaDetail {
    /// The schema name (e.g., `"BTMParameterEnum-145"`).
    pub name: String,
    /// Description from the schema, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Parent schema name from `allOf.$ref`, if this is a subtype.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Merged properties (own + inherited from parent via `allOf`).
    pub properties: Value,
    /// Required property names, if specified.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    /// Valid `btType` discriminator values, if this schema is polymorphic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtypes: Option<Vec<String>>,
    /// The discriminator property name (typically `"btType"`), if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discriminator_property: Option<String>,
}

/// A parsed and indexed `OpenAPI` specification.
#[derive(Debug)]
pub struct OpenApiSpec {
    /// The base server URL (e.g., `https://cad.onshape.com/api/v14`).
    server_url: String,
    /// All endpoints, indexed by operation ID.
    endpoints: HashMap<String, ParsedEndpoint>,
    /// Ordered list of operation IDs (for consistent search output).
    operation_ids: Vec<String>,
    /// Component schemas from the spec, for `$ref` resolution and schema lookup.
    components: HashMap<String, Value>,
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

    /// Parse an `OpenAPI` specification from a JSON string, using the provided
    /// server URL if the spec does not contain one.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is malformed or missing required fields.
    pub fn from_json_with_server_url_fallback(
        json: &str,
        server_url_fallback: &str,
    ) -> Result<Self, OpenApiError> {
        let root: Value = serde_json::from_str(json)?;
        Self::from_value_with_server_url_fallback(&root, server_url_fallback)
    }

    /// Parse an `OpenAPI` specification from a `serde_json::Value`.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is missing required fields.
    pub fn from_value(root: &Value) -> Result<Self, OpenApiError> {
        Self::from_value_inner(root, None)
    }

    /// Parse an `OpenAPI` specification from a `serde_json::Value`, using the
    /// provided server URL if the spec does not contain one.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is missing required fields.
    pub fn from_value_with_server_url_fallback(
        root: &Value,
        server_url_fallback: &str,
    ) -> Result<Self, OpenApiError> {
        Self::from_value_inner(root, Some(server_url_fallback))
    }

    fn from_value_inner(
        root: &Value,
        server_url_fallback: Option<&str>,
    ) -> Result<Self, OpenApiError> {
        // Extract server URL
        let server_url = root
            .pointer("/servers/0/url")
            .and_then(Value::as_str)
            .or(server_url_fallback)
            .ok_or_else(|| OpenApiError::InvalidSpec {
                reason: "missing 'servers[0].url' string".into(),
            })?
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
                let Ok(method) = method_str.parse::<HttpMethod>() else {
                    continue;
                };
                let Some(operation_id) = detail.get("operationId").and_then(Value::as_str) else {
                    continue;
                };

                let endpoint =
                    Self::parse_endpoint(operation_id, method, path, detail, &components);
                if endpoints
                    .insert(operation_id.to_string(), endpoint)
                    .is_none()
                {
                    operation_ids.push(operation_id.to_string());
                }
            }
        }

        Ok(Self {
            server_url,
            endpoints,
            operation_ids,
            components,
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

        let method_filter = filters
            .method
            .as_deref()
            .and_then(|s| s.parse::<HttpMethod>().ok());

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
                resolved_path = resolved_path
                    .replace(&format!("{{{}}}", param.name), &encode_path_param(value));
            }
        }

        // Also substitute any optional path params that were provided
        for (name, value) in path_params {
            resolved_path =
                resolved_path.replace(&format!("{{{name}}}"), &encode_path_param(value));
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

        for param in &ep.parameters {
            if param.location == ParameterLocation::Header && param.required {
                return Err(OpenApiError::InvalidParams {
                    reason: format!(
                        "required header parameter `{}` is unsupported because API requests do not model request headers yet",
                        param.name
                    ),
                });
            }
        }

        let query_params_vec: Vec<(String, String)> = query_params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let request_body = match body {
            Some(value) => {
                let is_multipart = ep
                    .request_body_content_type
                    .as_deref()
                    .is_some_and(|ct| ct.starts_with("multipart/form-data"));

                if is_multipart {
                    Some(Self::build_multipart_body(
                        value,
                        ep.request_body_schema.as_ref(),
                    )?)
                } else {
                    Some(RequestBody::Json(value))
                }
            }
            None => None,
        };

        Ok(ApiRequest {
            method: ep.method,
            path: resolved_path,
            query_params: query_params_vec,
            body: request_body,
            content_type: ep.request_body_content_type.clone(),
        })
    }

    /// Build a [`RequestBody::Multipart`] from a JSON [`Value`] body and the
    /// endpoint's request body schema.
    ///
    /// Fields whose schema declares `"format": "binary"` are treated as binary
    /// parts: their JSON string values are base64-decoded into raw bytes. All
    /// other fields become text parts with their JSON values converted to
    /// strings.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The body is not a JSON object
    /// - A binary field's value is not a string or is not valid base64
    fn build_multipart_body(
        body: Value,
        schema: Option<&Value>,
    ) -> Result<RequestBody, OpenApiError> {
        let Value::Object(fields) = body else {
            return Err(OpenApiError::InvalidParams {
                reason: "multipart/form-data body must be a JSON object".to_string(),
            });
        };

        let binary_field_names = Self::find_binary_fields(schema);
        let engine = base64::engine::general_purpose::STANDARD;

        let mut text_fields = Vec::new();
        let mut binary_fields = Vec::new();

        for (name, value) in fields {
            if binary_field_names.contains(&name) {
                let encoded = value.as_str().ok_or_else(|| OpenApiError::InvalidParams {
                    reason: format!(
                        "binary field `{name}` must be a base64-encoded string, got {}",
                        json_type_name(&value)
                    ),
                })?;
                let data = engine
                    .decode(encoded)
                    .map_err(|e| OpenApiError::InvalidParams {
                        reason: format!("binary field `{name}` has invalid base64: {e}"),
                    })?;
                binary_fields.push(BinaryField {
                    field_name: name,
                    data,
                    content_type: None,
                });
            } else if let Some(text) = json_value_to_text(&value) {
                // Non-binary field: convert JSON value to text for the form part.
                text_fields.push((name, text));
            }
            // Null values are skipped — omitting a field from the multipart form
            // is the correct representation of "not provided".
        }

        Ok(RequestBody::Multipart(MultipartBody {
            text_fields,
            binary_fields,
        }))
    }

    /// Inspect a request body schema's `properties` for fields with
    /// `"format": "binary"`, returning the set of property names.
    fn find_binary_fields(schema: Option<&Value>) -> HashSet<String> {
        let mut result = HashSet::new();
        let Some(schema) = schema else {
            return result;
        };
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return result;
        };

        for (name, prop) in properties {
            if prop.get("format").and_then(Value::as_str) == Some("binary") {
                result.insert(name.clone());
            }
        }

        result
    }

    /// Look up a component schema by name and return its detail.
    ///
    /// Merges parent properties (from `allOf.$ref`) into a flat `properties`
    /// object, annotates polymorphic `$ref` properties with `x-bttype-options`,
    /// and includes the discriminator subtypes if the schema is polymorphic.
    ///
    /// # Errors
    ///
    /// Returns an error if the schema name is not found in the spec's components.
    pub fn lookup_schema(&self, name: &str) -> Result<SchemaDetail, OpenApiError> {
        let schema = self
            .components
            .get(name)
            .ok_or_else(|| OpenApiError::SchemaNotFound {
                schema_name: name.to_string(),
            })?;

        let description = schema
            .get("description")
            .and_then(Value::as_str)
            .map(String::from);

        // Extract discriminator info from this schema.
        let discriminator = schema.get("discriminator");
        let discriminator_property = discriminator
            .and_then(|d| d.get("propertyName"))
            .and_then(Value::as_str)
            .map(String::from);
        let subtypes = discriminator
            .and_then(|d| d.get("mapping"))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect::<Vec<_>>());

        // Merge properties from allOf (parent) and own properties.
        let mut merged_props = serde_json::Map::new();
        let mut parent = None;
        let mut required: Vec<String> = Vec::new();

        // Collect required from the top-level schema.
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for r in req {
                if let Some(s) = r.as_str() {
                    required.push(s.to_string());
                }
            }
        }

        // Walk allOf to find parent ref and merge properties.
        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for item in all_of {
                if let Some(ref_str) = item.get("$ref").and_then(Value::as_str) {
                    // This is the parent reference.
                    if let Some(parent_name) = ref_str.strip_prefix("#/components/schemas/") {
                        parent = Some(parent_name.to_string());
                        // Merge parent properties (one level only — transitive
                        // ancestry is accessible via the `parent` field).
                        if let Some(parent_schema) = self.components.get(parent_name) {
                            Self::merge_props_and_required(
                                parent_schema,
                                &mut merged_props,
                                &mut required,
                            );
                            // Also merge properties/required from the parent's
                            // own allOf inline blocks (non-$ref items).  Many
                            // schemas in the Onshape spec carry their properties
                            // inside allOf rather than at the top level.
                            if let Some(parent_all_of) =
                                parent_schema.get("allOf").and_then(Value::as_array)
                            {
                                for parent_item in parent_all_of {
                                    if parent_item.get("$ref").is_some() {
                                        continue;
                                    }
                                    Self::merge_props_and_required(
                                        parent_item,
                                        &mut merged_props,
                                        &mut required,
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Inline properties/required from allOf item.
                    Self::merge_props_and_required(item, &mut merged_props, &mut required);
                }
            }
        }

        // Merge top-level properties (override parent if same key).
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, v) in props {
                merged_props.insert(k.clone(), v.clone());
            }
        }

        // Annotate properties that reference polymorphic schemas.
        let annotated =
            Self::annotate_discriminators(&Value::Object(merged_props), &self.components);
        let properties = annotated;

        Ok(SchemaDetail {
            name: name.to_string(),
            description,
            parent,
            properties,
            required,
            subtypes,
            discriminator_property,
        })
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    /// Merge `properties` and `required` from `source` into the accumulators.
    ///
    /// Used during `lookup_schema` to fold parent (and parent-allOf-inline)
    /// properties into the child's merged view.
    fn merge_props_and_required(
        source: &Value,
        merged_props: &mut serde_json::Map<String, Value>,
        required: &mut Vec<String>,
    ) {
        if let Some(props) = source.get("properties").and_then(Value::as_object) {
            for (k, v) in props {
                merged_props.insert(k.clone(), v.clone());
            }
        }
        if let Some(req) = source.get("required").and_then(Value::as_array) {
            for r in req {
                if let Some(s) = r.as_str()
                    && !required.contains(&s.to_string())
                {
                    required.push(s.to_string());
                }
            }
        }
    }

    /// Walk a schema's properties and annotate any `$ref` (or `items.$ref`)
    /// that points to a schema with a `discriminator.mapping` by adding an
    /// `x-bttype-options` array listing the valid btType values.
    ///
    /// Only examines one level of properties (does not recurse into subtypes).
    fn annotate_discriminators(schema: &Value, components: &HashMap<String, Value>) -> Value {
        let Some(props) = schema.as_object() else {
            return schema.clone();
        };

        let mut annotated = props.clone();

        for (key, value) in props {
            let annotated_value = Self::annotate_single_property(value, components);
            if annotated_value != *value {
                annotated.insert(key.clone(), annotated_value);
            }
        }

        Value::Object(annotated)
    }

    /// Check a single property value for `$ref` or `items.$ref` pointing to
    /// a schema with a discriminator, and annotate it with `x-bttype-options`.
    fn annotate_single_property(value: &Value, components: &HashMap<String, Value>) -> Value {
        // Direct $ref
        if let Some(ref_str) = value.get("$ref").and_then(Value::as_str)
            && let Some(options) = Self::discriminator_options(ref_str, components)
        {
            let mut annotated = value.as_object().cloned().unwrap_or_default();
            annotated.insert("x-bttype-options".to_string(), Value::Array(options));
            return Value::Object(annotated);
        }

        // items.$ref (for array properties)
        if let Some(items) = value.get("items")
            && let Some(ref_str) = items.get("$ref").and_then(Value::as_str)
            && let Some(options) = Self::discriminator_options(ref_str, components)
        {
            let mut annotated_items = items.as_object().cloned().unwrap_or_default();
            annotated_items.insert("x-bttype-options".to_string(), Value::Array(options));
            let mut annotated = value.as_object().cloned().unwrap_or_default();
            annotated.insert("items".to_string(), Value::Object(annotated_items));
            return Value::Object(annotated);
        }

        value.clone()
    }

    /// Annotate a resolved schema's `properties` with discriminator info.
    ///
    /// If the schema has a `properties` object, walk it and annotate any `$ref`
    /// properties that point to discriminator schemas. Returns the schema with
    /// the annotated properties in place.
    fn annotate_schema_properties(schema: &Value, components: &HashMap<String, Value>) -> Value {
        let mut result = schema.clone();
        let Some(obj) = result.as_object_mut() else {
            return result;
        };

        if let Some(props) = obj.get("properties").cloned() {
            obj.insert(
                "properties".to_string(),
                Self::annotate_discriminators(&props, components),
            );
        }

        if let Some(all_of) = obj.get_mut("allOf").and_then(Value::as_array_mut) {
            for item in all_of {
                let Some(props) = item.get("properties").cloned() else {
                    continue;
                };
                if let Some(item_obj) = item.as_object_mut() {
                    item_obj.insert(
                        "properties".to_string(),
                        Self::annotate_discriminators(&props, components),
                    );
                }
            }
        }

        result
    }

    /// Given a `$ref` string, check if the referenced schema has a discriminator
    /// mapping. If so, return the mapping keys as a `Vec<Value>` of strings.
    fn discriminator_options(
        ref_str: &str,
        components: &HashMap<String, Value>,
    ) -> Option<Vec<Value>> {
        let name = ref_str.strip_prefix("#/components/schemas/")?;
        let schema = components.get(name)?;
        let mapping = schema.get("discriminator")?.get("mapping")?.as_object()?;
        let options: Vec<Value> = mapping.keys().cloned().map(Value::String).collect();
        if options.is_empty() {
            None
        } else {
            Some(options)
        }
    }

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
            // Prefer application/json (including variants like
            // "application/json;charset=UTF-8; qs=0.09"); fall back to the
            // first content type.
            let entry = Self::prefer_json_content(content_map);
            if let Some((content_type, schema_info)) = entry {
                let schema = schema_info.get("schema").cloned();
                let resolved = schema.map(|s| {
                    let resolved = Self::resolve_ref_shallow(&s, components);
                    Self::annotate_schema_properties(&resolved, components)
                });
                return (true, resolved, Some(content_type.to_string()));
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
        // Prefer application/json (including variants); fall back to the
        // first content type.
        let (_, schema_info) = Self::prefer_json_content(content)?;
        let schema = schema_info.get("schema")?;

        let resolved = Self::resolve_ref_shallow(schema, components);
        Some(Self::annotate_schema_properties(&resolved, components))
    }

    /// Pick the `application/json` (or `application/json;…` variant) entry from
    /// a content-type map, falling back to the first entry if no JSON type is
    /// found.
    fn prefer_json_content(content_map: &serde_json::Map<String, Value>) -> Option<(&str, &Value)> {
        content_map
            .iter()
            .find(|(k, _)| k.starts_with("application/json"))
            .map(|(k, v)| (k.as_str(), v))
            .or_else(|| content_map.iter().next().map(|(k, v)| (k.as_str(), v)))
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
            let mut end = max_len.saturating_sub(3);
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &s[..end])
        }
    }
}

/// Percent-encode a path parameter value to ensure it remains a single path segment.
///
/// Encodes everything except unreserved characters (RFC 3986 §2.3: ALPHA, DIGIT,
/// `-`, `.`, `_`, `~`). This prevents path traversal and URL structure alteration
/// from user-provided values.
fn encode_path_param(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Convert a JSON value to its text representation for a multipart text part.
///
/// Returns `None` for `Value::Null` (null fields are omitted from the form).
fn json_value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        // Arrays and objects: serialize as JSON strings for the text part.
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).ok(),
    }
}

/// Return a human-readable name for a JSON value type (for error messages).
const fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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
                                "application/json;charset=UTF-8; qs=0.09": {
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
                },
                "/partstudios/features": {
                    "post": {
                        "operationId": "addFeature",
                        "summary": "Add a feature to a part studio",
                        "tags": ["PartStudio"],
                        "requestBody": {
                            "content": {
                                "application/json;charset=UTF-8; qs=0.09": {
                                    "schema": { "$ref": "#/components/schemas/BTFeatureDefinitionCall-1406" }
                                }
                            }
                        },
                        "responses": { "200": {} }
                    }
                },
                "/blobelements/d/{did}/w/{wid}": {
                    "post": {
                        "operationId": "uploadFileCreateElement",
                        "summary": "Upload a file to create a new element",
                        "tags": ["BlobElement"],
                        "parameters": [
                            {
                                "name": "did",
                                "in": "path",
                                "required": true,
                                "schema": { "type": "string" },
                                "description": "Document ID"
                            },
                            {
                                "name": "wid",
                                "in": "path",
                                "required": true,
                                "schema": { "type": "string" },
                                "description": "Workspace ID"
                            }
                        ],
                        "requestBody": {
                            "content": {
                                "multipart/form-data": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "file": {
                                                "type": "string",
                                                "format": "binary",
                                                "description": "The file to upload."
                                            },
                                            "formatName": { "type": "string" },
                                            "translate": { "type": "boolean" },
                                            "encodedFilename": { "type": "string" }
                                        }
                                    }
                                }
                            }
                        },
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
                    },
                    "BTMFeature-134": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string", "description": "Type of JSON object." },
                            "featureId": { "type": "string" },
                            "name": { "type": "string" },
                            "parameters": {
                                "type": "array",
                                "items": { "$ref": "#/components/schemas/BTMParameter-1" }
                            }
                        },
                        "discriminator": {
                            "propertyName": "btType",
                            "mapping": {
                                "BTMSketch-151": "#/components/schemas/BTMSketch-151",
                                "BTMFeatureInvalid-1031": "#/components/schemas/BTMFeatureInvalid-1031"
                            }
                        }
                    },
                    "BTMSketch-151": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTMFeature-134" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "constraints": { "type": "array" },
                                    "entities": { "type": "array" }
                                }
                            }
                        ]
                    },
                    "BTMParameter-1": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string", "description": "Type of JSON object." },
                            "parameterId": { "type": "string" },
                            "parameterName": { "type": "string" }
                        },
                        "description": "A parameter value.",
                        "discriminator": {
                            "propertyName": "btType",
                            "mapping": {
                                "BTMParameterEnum-145": "#/components/schemas/BTMParameterEnum-145",
                                "BTMParameterQuantity-147": "#/components/schemas/BTMParameterQuantity-147",
                                "BTMParameterString-149": "#/components/schemas/BTMParameterString-149"
                            }
                        }
                    },
                    "BTMParameterEnum-145": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTMParameter-1" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "enumName": { "type": "string" },
                                    "value": { "type": "string" }
                                }
                            }
                        ]
                    },
                    "BTMParameterQuantity-147": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTMParameter-1" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "expression": { "type": "string" },
                                    "value": { "type": "number" }
                                }
                            }
                        ]
                    },
                    "BTMParameterString-149": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTMParameter-1" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "value": { "type": "string" }
                                }
                            }
                        ]
                    },
                    "BTFeatureDefinitionCall-1406": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" },
                            "feature": { "$ref": "#/components/schemas/BTMFeature-134" },
                            "libraryVersion": { "type": "integer" }
                        }
                    },
                    "BTMFeatureInvalid-1031": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTMFeature-134" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "reason": { "type": "string" }
                                }
                            }
                        ]
                    },
                    "BTGrandparent-50": {
                        "type": "object",
                        "properties": {
                            "grandparentProp": { "type": "string" }
                        }
                    },
                    "BTParentWithAllOfProps-100": {
                        "type": "object",
                        "discriminator": {
                            "propertyName": "btType",
                            "mapping": {
                                "BTChildOfAllOfParent-200": "#/components/schemas/BTChildOfAllOfParent-200"
                            }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTGrandparent-50" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "parentInlineProp": { "type": "integer" }
                                },
                                "required": ["parentInlineProp"]
                            }
                        ]
                    },
                    "BTChildOfAllOfParent-200": {
                        "type": "object",
                        "properties": {
                            "btType": { "type": "string" }
                        },
                        "allOf": [
                            { "$ref": "#/components/schemas/BTParentWithAllOfProps-100" },
                            {
                                "type": "object",
                                "properties": {
                                    "btType": { "type": "string" },
                                    "childOwnProp": { "type": "boolean" }
                                }
                            }
                        ]
                    }
                }
            }
        }"##
    }

    #[test]
    fn parse_spec() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        assert_eq!(spec.endpoint_count(), 7);
        assert_eq!(spec.server_url(), "https://example.com/api/v1");
    }

    #[test]
    fn parse_requires_server_url_without_fallback() {
        let err = OpenApiSpec::from_value(&serde_json::json!({
            "openapi": "3.0.1",
            "paths": {}
        }))
        .unwrap_err();

        match err {
            OpenApiError::InvalidSpec { reason } => {
                assert!(
                    reason.contains("missing 'servers[0].url' string"),
                    "error should mention missing server URL, got: {reason}"
                );
            }
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    #[test]
    fn parse_accepts_explicit_server_url_fallback() {
        let spec = OpenApiSpec::from_value_with_server_url_fallback(
            &serde_json::json!({
                "openapi": "3.0.1",
                "paths": {}
            }),
            "https://fallback.example.com/api/v1",
        )
        .expect("should parse");

        assert_eq!(spec.server_url(), "https://fallback.example.com/api/v1");
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
        assert_eq!(results.len(), 7);
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
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.operation_id == "listParts"));
        assert!(results.iter().any(|r| r.operation_id == "addFeature"));
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
        assert_eq!(
            detail.request_body_content_type.as_deref(),
            Some("application/json;charset=UTF-8; qs=0.09")
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
        path_params.insert("did".to_string(), "abc/123 model".to_string());

        let request = spec
            .build_request("getDocument", &path_params, &HashMap::new(), None)
            .expect("should build");

        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/documents/abc%2F123%20model");
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
        query_params.insert("q".to_string(), "robot arm".to_string());
        query_params.insert("limit".to_string(), "10".to_string());

        let request = spec
            .build_request("getDocuments", &HashMap::new(), &query_params, None)
            .expect("should build");

        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/documents");
        assert_eq!(request.query_params.len(), 2);

        let query_map: HashMap<&str, &str> = request
            .query_params
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert_eq!(query_map["q"], "robot arm");
        assert_eq!(query_map["limit"], "10");
    }

    #[test]
    fn build_request_rejects_missing_required_query_param() {
        let spec = OpenApiSpec::from_value(&serde_json::json!({
            "openapi": "3.0.1",
            "servers": [{ "url": "https://example.com/api/v1" }],
            "paths": {
                "/search": {
                    "get": {
                        "operationId": "searchDocuments",
                        "parameters": [
                            {
                                "name": "q",
                                "in": "query",
                                "required": true,
                                "schema": { "type": "string" }
                            }
                        ],
                        "responses": { "200": { "description": "ok" } }
                    }
                }
            }
        }))
        .expect("should parse");

        let err = spec
            .build_request("searchDocuments", &HashMap::new(), &HashMap::new(), None)
            .unwrap_err();

        match err {
            OpenApiError::InvalidParams { reason } => {
                assert!(
                    reason.contains("missing required query parameter: q"),
                    "error should mention missing required query parameter, got: {reason}"
                );
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn build_request_rejects_required_header_params() {
        let spec = OpenApiSpec::from_value(&serde_json::json!({
            "openapi": "3.0.1",
            "servers": [{ "url": "https://example.com/api/v1" }],
            "paths": {
                "/downloads/{did}": {
                    "get": {
                        "operationId": "downloadExternalData",
                        "parameters": [
                            {
                                "name": "did",
                                "in": "path",
                                "required": true,
                                "schema": { "type": "string" }
                            },
                            {
                                "name": "If-Match",
                                "in": "header",
                                "required": true,
                                "schema": { "type": "string" }
                            }
                        ],
                        "responses": { "200": { "description": "ok" } }
                    }
                }
            }
        }))
        .expect("should parse");
        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());

        let err = spec
            .build_request("downloadExternalData", &path_params, &HashMap::new(), None)
            .unwrap_err();

        match err {
            OpenApiError::InvalidParams { reason } => {
                assert!(
                    reason.contains("required header parameter `If-Match` is unsupported"),
                    "error should mention unsupported required header, got: {reason}"
                );
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
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
        assert!(
            matches!(request.body, Some(RequestBody::Json(_))),
            "JSON endpoint should produce RequestBody::Json"
        );
        assert_eq!(
            request.content_type.as_deref(),
            Some("application/json;charset=UTF-8; qs=0.09")
        );
    }

    #[test]
    fn invalid_json_returns_error() {
        let err = OpenApiSpec::from_json("not json").unwrap_err();
        assert!(matches!(err, OpenApiError::ParseError(_)));
    }

    #[test]
    fn missing_paths_returns_error() {
        let err = OpenApiSpec::from_json(
            r#"{"openapi": "3.0.1", "servers": [{ "url": "https://example.com/api/v1" }]}"#,
        )
        .unwrap_err();
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

    // ====================================================================
    // Discriminator Annotation Tests
    // ====================================================================

    #[test]
    fn explain_annotates_discriminator_refs_in_request_body() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec.explain("addFeature").expect("should find");

        let schema = detail
            .request_body_schema
            .expect("should have request body schema");
        let props = schema.get("properties").expect("should have properties");

        // The "feature" property refs BTMFeature-134 which has a discriminator.
        let feature = props.get("feature").expect("should have feature property");
        let options = feature
            .get("x-bttype-options")
            .expect("should have x-bttype-options annotation");
        let options_arr = options.as_array().expect("should be an array");
        assert!(options_arr.len() >= 2);
        assert!(options_arr.contains(&Value::String("BTMSketch-151".to_string())));
        assert!(options_arr.contains(&Value::String("BTMFeatureInvalid-1031".to_string())));
    }

    #[test]
    fn explain_annotates_discriminator_refs_in_array_items() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");

        // BTMFeature-134 has "parameters" which is an array of BTMParameter-1 refs.
        // When addFeature is explained, the resolved BTFeatureDefinitionCall-1406
        // should show annotations on the feature ref... but BTMFeature-134's own
        // properties aren't resolved in the explain output (only one level of ref
        // resolution). So let's use lookup_schema to check array items annotation.
        let detail = spec
            .lookup_schema("BTMFeature-134")
            .expect("should find schema");
        let props = detail.properties.as_object().expect("should be object");
        let params = props.get("parameters").expect("should have parameters");
        let items = params.get("items").expect("should have items");
        let options = items
            .get("x-bttype-options")
            .expect("items should have x-bttype-options");
        let options_arr = options.as_array().expect("should be an array");
        assert!(options_arr.contains(&Value::String("BTMParameterEnum-145".to_string())));
        assert!(options_arr.contains(&Value::String("BTMParameterQuantity-147".to_string())));
        assert!(options_arr.contains(&Value::String("BTMParameterString-149".to_string())));
    }

    #[test]
    fn explain_does_not_annotate_non_discriminator_refs() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec.explain("getDocuments").expect("should find");

        // DocumentList has no discriminator references — should have no annotations.
        if let Some(schema) = detail.response_schema
            && let Some(props) = schema.get("properties")
            && let Some(obj) = props.as_object()
        {
            for (_key, value) in obj {
                assert!(
                    value.get("x-bttype-options").is_none(),
                    "non-discriminator properties should not be annotated"
                );
            }
        }
    }

    // ====================================================================
    // Schema Lookup Tests
    // ====================================================================

    #[test]
    fn lookup_schema_parent_type_has_subtypes() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec
            .lookup_schema("BTMParameter-1")
            .expect("should find schema");

        assert_eq!(detail.name, "BTMParameter-1");
        assert_eq!(detail.description.as_deref(), Some("A parameter value."));
        assert!(detail.parent.is_none(), "base type should have no parent");
        assert_eq!(detail.discriminator_property.as_deref(), Some("btType"));

        let subtypes = detail.subtypes.expect("should have subtypes");
        assert_eq!(subtypes.len(), 3);
        assert!(subtypes.contains(&"BTMParameterEnum-145".to_string()));
        assert!(subtypes.contains(&"BTMParameterQuantity-147".to_string()));
        assert!(subtypes.contains(&"BTMParameterString-149".to_string()));
    }

    #[test]
    fn lookup_schema_subtype_merges_parent_properties() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec
            .lookup_schema("BTMParameterEnum-145")
            .expect("should find schema");

        assert_eq!(detail.name, "BTMParameterEnum-145");
        assert_eq!(detail.parent.as_deref(), Some("BTMParameter-1"));
        assert!(
            detail.subtypes.is_none(),
            "leaf type should have no subtypes"
        );

        let props = detail.properties.as_object().expect("should be object");
        // Own properties
        assert!(
            props.contains_key("enumName"),
            "should have own property enumName"
        );
        assert!(
            props.contains_key("value"),
            "should have own property value"
        );
        // Inherited properties from BTMParameter-1
        assert!(
            props.contains_key("parameterId"),
            "should have inherited property parameterId"
        );
        assert!(
            props.contains_key("parameterName"),
            "should have inherited property parameterName"
        );
        assert!(props.contains_key("btType"), "should have btType property");
    }

    #[test]
    fn lookup_schema_nonexistent_returns_error() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let err = spec.lookup_schema("NonExistent-999").unwrap_err();
        assert!(matches!(err, OpenApiError::SchemaNotFound { .. }));
    }

    #[test]
    fn lookup_schema_non_subtype_has_own_properties() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec
            .lookup_schema("BTFeatureDefinitionCall-1406")
            .expect("should find schema");

        assert!(detail.parent.is_none());
        let props = detail.properties.as_object().expect("should be object");
        assert!(props.contains_key("btType"));
        assert!(props.contains_key("feature"));
        assert!(props.contains_key("libraryVersion"));

        // The "feature" property should be annotated with x-bttype-options
        let feature = props.get("feature").expect("should have feature");
        assert!(
            feature.get("x-bttype-options").is_some(),
            "feature ref should be annotated with discriminator options"
        );
    }

    #[test]
    fn lookup_schema_subtype_with_discriminator_has_subtypes() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec
            .lookup_schema("BTMFeature-134")
            .expect("should find schema");

        assert_eq!(detail.discriminator_property.as_deref(), Some("btType"));
        let subtypes = detail.subtypes.expect("should have subtypes");
        assert!(subtypes.contains(&"BTMSketch-151".to_string()));
        assert!(subtypes.contains(&"BTMFeatureInvalid-1031".to_string()));
    }

    #[test]
    fn lookup_schema_merges_parent_allof_inline_properties() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let detail = spec
            .lookup_schema("BTChildOfAllOfParent-200")
            .expect("should find schema");

        assert_eq!(detail.parent.as_deref(), Some("BTParentWithAllOfProps-100"));

        let props = detail.properties.as_object().expect("should be object");
        // Own property from the child's allOf inline block.
        assert!(
            props.contains_key("childOwnProp"),
            "should have own property childOwnProp"
        );
        // Property from the parent's allOf inline block (not at the
        // parent's top level).
        assert!(
            props.contains_key("parentInlineProp"),
            "should have parent's allOf inline property parentInlineProp"
        );
        assert!(
            props.contains_key("btType"),
            "should have btType from parent"
        );

        // Parent required should be merged.
        assert!(
            detail.required.contains(&"parentInlineProp".to_string()),
            "should inherit required from parent's allOf inline block"
        );
    }

    // ====================================================================
    // Multipart Body Tests
    // ====================================================================

    #[test]
    fn build_request_multipart_produces_multipart_body() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let engine = base64::engine::general_purpose::STANDARD;
        let file_content = b"hello world";
        let encoded = base64::Engine::encode(&engine, file_content);

        let body = serde_json::json!({
            "file": encoded,
            "formatName": "FEATURESCRIPT",
            "translate": true,
            "encodedFilename": "test.fs",
            "importAppearances": { "faces": true }
        });

        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());
        path_params.insert("wid".to_string(), "ws1".to_string());

        let request = spec
            .build_request(
                "uploadFileCreateElement",
                &path_params,
                &HashMap::new(),
                Some(body),
            )
            .expect("should build");

        assert_eq!(request.content_type.as_deref(), Some("multipart/form-data"));

        let multipart = match request.body {
            Some(RequestBody::Multipart(m)) => m,
            other => panic!("expected Multipart body, got {other:?}"),
        };

        // Check binary field.
        assert_eq!(multipart.binary_fields.len(), 1);
        assert_eq!(multipart.binary_fields[0].field_name, "file");
        assert_eq!(multipart.binary_fields[0].data, file_content);

        // Check text fields.
        assert_eq!(multipart.text_fields.len(), 4);
        let text_map: HashMap<&str, &str> = multipart
            .text_fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(text_map["formatName"], "FEATURESCRIPT");
        assert_eq!(text_map["translate"], "true");
        assert_eq!(text_map["encodedFilename"], "test.fs");
        assert_eq!(text_map["importAppearances"], r#"{"faces":true}"#);
    }

    #[test]
    fn build_request_multipart_invalid_base64_returns_error() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let body = serde_json::json!({
            "file": "not-valid-base64!!!",
            "formatName": "STEP"
        });

        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());
        path_params.insert("wid".to_string(), "ws1".to_string());

        let err = spec
            .build_request(
                "uploadFileCreateElement",
                &path_params,
                &HashMap::new(),
                Some(body),
            )
            .unwrap_err();

        match err {
            OpenApiError::InvalidParams { reason } => {
                assert!(
                    reason.contains("invalid base64"),
                    "error should mention invalid base64, got: {reason}"
                );
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn build_request_multipart_non_string_binary_field_returns_error() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let body = serde_json::json!({
            "file": 42,
            "formatName": "STEP"
        });

        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());
        path_params.insert("wid".to_string(), "ws1".to_string());

        let err = spec
            .build_request(
                "uploadFileCreateElement",
                &path_params,
                &HashMap::new(),
                Some(body),
            )
            .unwrap_err();

        match err {
            OpenApiError::InvalidParams { reason } => {
                assert!(
                    reason.contains("base64-encoded string"),
                    "error should mention expected string, got: {reason}"
                );
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn build_request_multipart_non_object_body_returns_error() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let body = serde_json::json!("just a string");

        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());
        path_params.insert("wid".to_string(), "ws1".to_string());

        let err = spec
            .build_request(
                "uploadFileCreateElement",
                &path_params,
                &HashMap::new(),
                Some(body),
            )
            .unwrap_err();

        match err {
            OpenApiError::InvalidParams { reason } => {
                assert!(
                    reason.contains("JSON object"),
                    "error should mention JSON object, got: {reason}"
                );
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn build_request_multipart_null_fields_are_skipped() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let engine = base64::engine::general_purpose::STANDARD;
        let body = serde_json::json!({
            "file": base64::Engine::encode(&engine, b"data"),
            "formatName": null
        });

        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());
        path_params.insert("wid".to_string(), "ws1".to_string());

        let request = spec
            .build_request(
                "uploadFileCreateElement",
                &path_params,
                &HashMap::new(),
                Some(body),
            )
            .expect("should build");

        let multipart = match request.body {
            Some(RequestBody::Multipart(m)) => m,
            other => panic!("expected Multipart body, got {other:?}"),
        };

        // formatName is null, should be omitted from text fields.
        assert!(
            !multipart.text_fields.iter().any(|(k, _)| k == "formatName"),
            "null field should be omitted"
        );
    }

    #[test]
    fn build_request_multipart_no_body_passes_through_as_none() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");

        let mut path_params = HashMap::new();
        path_params.insert("did".to_string(), "doc1".to_string());
        path_params.insert("wid".to_string(), "ws1".to_string());

        let request = spec
            .build_request(
                "uploadFileCreateElement",
                &path_params,
                &HashMap::new(),
                None,
            )
            .expect("should build");

        assert!(request.body.is_none());
    }

    #[test]
    fn build_request_json_endpoint_still_produces_json_body() {
        let spec = OpenApiSpec::from_json(test_spec_json()).expect("should parse");
        let body = serde_json::json!({"name": "My Document"});

        let request = spec
            .build_request(
                "createDocument",
                &HashMap::new(),
                &HashMap::new(),
                Some(body.clone()),
            )
            .expect("should build");

        match request.body {
            Some(RequestBody::Json(v)) => {
                assert_eq!(v, body);
            }
            other => panic!("expected Json body, got {other:?}"),
        }
    }

    #[test]
    fn find_binary_fields_from_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "file": { "type": "string", "format": "binary" },
                "name": { "type": "string" },
                "count": { "type": "integer" }
            }
        });

        let result = OpenApiSpec::find_binary_fields(Some(&schema));
        assert_eq!(result.len(), 1);
        assert!(result.contains("file"));
    }

    #[test]
    fn find_binary_fields_none_schema_returns_empty() {
        let result = OpenApiSpec::find_binary_fields(None);
        assert!(result.is_empty());
    }

    #[test]
    fn json_value_to_text_conversions() {
        assert_eq!(super::json_value_to_text(&Value::Null), None);
        assert_eq!(
            super::json_value_to_text(&Value::Bool(true)),
            Some("true".to_string())
        );
        assert_eq!(
            super::json_value_to_text(&Value::Bool(false)),
            Some("false".to_string())
        );
        assert_eq!(
            super::json_value_to_text(&Value::from(42)),
            Some("42".to_string())
        );
        assert_eq!(
            super::json_value_to_text(&Value::from(2.75)),
            Some("2.75".to_string())
        );
        assert_eq!(
            super::json_value_to_text(&Value::from("hello")),
            Some("hello".to_string())
        );
    }
}
