//! Generic input types for API tools; hosts configure the advertised wording.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::FileReference;

/// Input schema for the API search tool.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ApiSearchInput {
    /// Free-text search query. Matches against endpoint names, paths,
    /// descriptions, and tags. Leave empty to list all endpoints.
    pub query: String,
    /// Filter by HTTP method (e.g., "GET", "POST", "DELETE").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Filter by tag name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// Input schema for the API explain tool.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ApiExplainInput {
    /// The operation ID of the endpoint to explain (from search results).
    pub endpoint: String,
}

/// Input schema for the API call tool.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ApiCallInput {
    /// The operation ID of the endpoint to call.
    pub endpoint: String,
    /// Path parameters keyed by the names in the endpoint path template.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub path_params: HashMap<String, String>,
    /// Query parameters keyed by parameter name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub query_params: HashMap<String, String>,
    /// Header parameters (e.g., `{"Accept": "application/octet-stream"}`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub header_params: HashMap<String, String>,
    /// JSON value for the request body (for POST/PUT/PATCH endpoints).
    /// Legacy serialized JSON strings are also accepted for compatibility.
    /// Explain the endpoint to see its expected request schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// File references for fields whose content should be read from disk.
    ///
    /// Each reference specifies a file path, a body field name, and an encoding.
    /// The server reads the files and injects their content into the request body
    /// after building the request. Fields listed here should be omitted from `body`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_refs: Vec<FileReference>,
}

/// Input schema for the API schema tool.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ApiSchemaInput {
    /// The component schema name to look up, taken from an endpoint explanation
    /// or a previous schema lookup.
    pub schema: String,
}
