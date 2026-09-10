//! Onshape names and guidance for the generic API tools.

use std::sync::OnceLock;

use super::api::{ToolDefinition, ToolKind, ToolSet};

/// The shared source of Onshape API tool discovery metadata and dispatch names.
#[allow(clippy::expect_used)] // Static host configuration is verified by the metadata regression test.
pub fn api_tools() -> &'static ToolSet {
    static TOOLS: OnceLock<ToolSet> = OnceLock::new();
    TOOLS.get_or_init(|| {
        ToolSet::new(&[
            ToolDefinition {
                kind: ToolKind::Search,
                name: "onshape_api_search",
                description: concat!(
                    "Find Onshape API endpoints by keyword or filter. Returns brief summaries (endpoint ID, ",
                    "method, path template, one-line description). Use this to discover available endpoints ",
                    "before calling onshape_api_explain for details.",
                ),
                input_description: Some("Input schema for `onshape_api_search`."),
                field_descriptions: &[(
                    "tag",
                    "Filter by tag name (e.g., \"Document\", \"Assembly\", \"`PartStudio`\").",
                )],
            },
            ToolDefinition {
                kind: ToolKind::Explain,
                name: "onshape_api_explain",
                description: concat!(
                    "Get full details for a specific Onshape API endpoint. Returns parameter schemas, types, ",
                    "required/optional flags, request/response schemas. Use the endpoint's operationId from ",
                    "onshape_api_search results.",
                ),
                input_description: Some("Input schema for `onshape_api_explain`."),
                field_descriptions: &[],
            },
            ToolDefinition {
                kind: ToolKind::Call,
                name: "onshape_api_call",
                description: concat!(
                    "Invoke an Onshape API endpoint. Provide the operationId and structured parameters ",
                    "(path_params, query_params, body). Path parameters are named fields (e.g., {\"did\": ",
                    "\"abc123\"}), not baked into a URL string. For endpoints that accept file content (e.g., ",
                    "file uploads), use `file_refs` to reference local files instead of inlining content in ",
                    "the body — the server reads them directly. Returns the API response.",
                ),
                input_description: Some("Input schema for `onshape_api_call`."),
                field_descriptions: &[
                    (
                        "path_params",
                        "Path parameters (e.g., `{\"did\": \"abc123\", \"wid\": \"def456\"}`).",
                    ),
                    (
                        "query_params",
                        "Query parameters (e.g., `{\"q\": \"robot arm\", \"limit\": \"10\"}`).",
                    ),
                    (
                        "body",
                        concat!(
                            "JSON value for the request body (for POST/PUT/PATCH endpoints).\n",
                            "Legacy serialized JSON strings are also accepted for compatibility. Use\n",
                            "`onshape_api_explain` to see the expected schema for each endpoint.",
                        ),
                    ),
                    (
                        "file_refs",
                        concat!(
                            "File references for fields whose content should be read from disk.\n",
                            "\n",
                            "Each reference specifies a file path, a body field name, and an encoding.\n",
                            "The server reads the files and injects their content into the request body\n",
                            "after building the request. Fields listed here should be omitted from `body`.\n",
                            "\n",
                            "Example: to upload a file via `uploadFileCreateElement`, pass the metadata\n",
                            "fields in `body` and use `file_refs` for the binary content:\n",
                            "`body: {\"formatName\": \"PARASOLID\"}`,\n",
                            "`file_refs: [{\"path\": \"/tmp/part.x_t\", \"field\": \"file\", \"encoding\": \"raw_bytes\"}]`",
                        ),
                    ),
                ],
            },
            ToolDefinition {
                kind: ToolKind::Schema,
                name: "onshape_api_schema",
                description: concat!(
                    "Look up an Onshape API schema by name. Returns the schema's properties (merged with ",
                    "inherited parent properties), discriminator subtypes if polymorphic, and parent type. Use ",
                    "schema names from x-bttype-options annotations in onshape_api_explain results to drill ",
                    "into specific types.",
                ),
                input_description: Some("Input schema for `onshape_api_schema`."),
                field_descriptions: &[(
                    "schema",
                    concat!(
                        "The schema name to look up (e.g., `\"BTMParameterEnum-145\"` or\n",
                        "`\"BTFeatureDefinitionCall-1406\"`). Use schema names from\n",
                        "`x-bttype-options` annotations in `onshape_api_explain` results,\n",
                        "or from the `subtypes` field in previous `onshape_api_schema` results.",
                    ),
                )],
            },
        ])
        .expect("Onshape API tool configuration should be valid")
    })
}
