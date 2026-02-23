# MCP Tools

## Primary Use Cases

1. **Exporting** — Get data out of Onshape (STL, STEP, glTF, etc.) via the generic API tool
2. **Exploring** — Navigate and understand existing designs
3. **AI-assisted FeatureScript development** — Later phase

## Target Users

Individuals first, with architecture that doesn't preclude teams.

## Tool Naming Convention

All MCP tools use the `onshape_` prefix to avoid collisions with other MCP servers.

| Prefix | Purpose | Example |
| -------- | --------- | --------- |
| `onshape_api_` | Onshape REST API operations | `onshape_api_search` |
| `onshape_mcp_` | MCP server administration | `onshape_mcp_auth_status` |

## Transport Support

| Transport | Priority | Notes |
| ----------- | ---------- | ------- |
| stdio | P0 | Primary MCP transport |
| HTTP/SSE | P1 | Server-Sent Events |
| WebSocket | P2 | Bidirectional |

## Permission Model

The server supports three permission modes controlling which tools are visible and callable.

### Modes

| Mode | Tools Available | Description |
| ------ | ----------------- | ------------- |
| `read` | Read-only tools | GET endpoints (query, list, export) |
| `modify` | Read + non-destructive writes | GET, POST, PUT, PATCH endpoints |
| `destroy` | All tools | All endpoints including DELETE |

### Mode Configuration

Mode settings are configured via the standard configuration system. See [Configuration](configuration.md#all-settings-reference) for details on `max_mode`, `initial_mode`, and `allow_mode_escalation`.

**Why explicit `allow_mode_escalation`?** Without it, we cannot distinguish between:

- User wants AI to escalate when needed (interactive)
- User set max_mode as ceiling but controls mode manually per session

### Permission Enforcement

Permission enforcement will happen at call time in `onshape_api_call`. Currently, the tool validates inputs and builds a complete API request using the OpenAPI spec, but HTTP execution is pending (the `onshape-client-io` crate is not yet built). Permission mode checks will be added alongside HTTP execution. The search and explain tools return information about all endpoints regardless of permission mode (they don't modify anything). The call tool will reject operations that exceed the current mode, with a clear error message.

### MCP Tool Annotations

Tools declare their characteristics using MCP's `ToolAnnotations`:

- `readOnlyHint` — true if tool doesn't modify Onshape data
- `destructiveHint` — true if tool may perform destructive operations

These are advisory hints for MCP clients, not security enforcement.

| Tool | `readOnlyHint` | `destructiveHint` |
| ---- | -------------- | ----------------- |
| `onshape_mcp_auth_status` | true | false |
| `onshape_api_search` | true | false |
| `onshape_api_explain` | true | false |
| `onshape_api_call` | false | true |

## Server Administration Tools

Always visible (read-only operations on the server itself).

| Tool | Description | Status |
| ------ | ------------- | ------ |
| `onshape_mcp_get_mode` | Returns current mode, max mode, escalation allowed | Not yet implemented |
| `onshape_mcp_request_mode` | Request mode change (escalate or de-escalate, within max) | Not yet implemented |
| `onshape_mcp_auth_status` | Returns auth status (valid/invalid/expired), last check time, connectivity | Implemented |

## Onshape API Tools

Instead of individual tools for each API endpoint (which would consume too much LLM context), the server provides three generic tools powered by the Onshape OpenAPI specification.

### Design Rationale

MCP clients enumerate all tools in the system prompt. With 30+ individual tools, the schema definitions alone would consume significant context. Three generic tools keep the footprint minimal while providing access to the full Onshape API.

### Tool Overview

| Tool | Purpose |
| ------ | --------- |
| `onshape_api_search` | Find Onshape API endpoints by keyword or filter |
| `onshape_api_explain` | Get full details for a specific endpoint |
| `onshape_api_call` | Invoke an Onshape API endpoint with structured parameters |

### Workflow

An LLM uses these tools in a natural progression:

1. **Search** to find relevant endpoints
2. **Explain** to learn the parameters for a specific endpoint
3. **Call** to execute it

### `onshape_api_search`

Find Onshape API endpoints by keyword or filter. Returns brief summaries (endpoint ID, method, path template, one-line description).

**Input:**

| Parameter | Type | Required | Description |
| ----------- | -------- | ---------- | ------------- |
| `query` | `string` | Yes | Free-text search query. Matches against endpoint names, paths, descriptions, and tags. Empty string returns all endpoints. |
| `method` | `string` | No | Filter by HTTP method (e.g., "GET", "POST", "DELETE") |
| `tag` | `string` | No | Filter by tag name (e.g., "Document", "Assembly", "PartStudio") |

**Output:** JSON array of endpoint summaries:

```json
[
  {
    "operation_id": "getDocuments",
    "method": "GET",
    "path": "/documents",
    "description": "Get a list of documents.",
    "tags": ["Document"]
  }
]
```

### `onshape_api_explain`

Get full details for a specific endpoint. Returns parameter schemas, types, required/optional flags, request/response schemas.

**Input:**

| Parameter | Type | Required | Description |
| ----------- | -------- | ---------- | ------------- |
| `endpoint` | `string` | Yes | The operation ID from search results |

**Output:** JSON object with full endpoint detail:

```json
{
  "operation_id": "getDocument",
  "method": "GET",
  "path": "/documents/{did}",
  "description": "Get document metadata.",
  "tags": ["Document"],
  "parameters": [
    {
      "name": "did",
      "location": "path",
      "required": true,
      "param_type": "string",
      "description": "Document ID"
    }
  ],
  "has_request_body": false,
  "response_schema": { "..." : "..." }
}
```

Fields `default`, `enum_values`, `request_body_schema`, and `request_body_content_type` are omitted from the response when null (via `skip_serializing_if`).
They appear only when the endpoint has relevant values (e.g., POST/PUT endpoints with a request body, or parameters with defaults or enum constraints).

### `onshape_api_call`

Invoke an Onshape API endpoint with structured parameters. Path parameters are named fields, not baked into a URL string. The tool resolves the path template internally using the spec.

**Input:**

| Parameter | Type | Required | Description |
| ----------- | -------- | ---------- | ------------- |
| `endpoint` | `string` | Yes | The operation ID to call |
| `path_params` | `object` | No | Path parameters (e.g., `{"did": "abc123"}`) |
| `query_params` | `object` | No | Query parameters (e.g., `{"q": "robot arm", "limit": "10"}`) |
| `body` | `any` | No | Request body (for POST/PUT/PATCH endpoints) |

**Output:** The API response content.

**Effect Pattern:** This tool uses the effects-as-data pattern. The core crate validates the request and produces an `ApiRequest` effect, which the I/O layer executes. This keeps the core crate sans-IO.

### OpenAPI Spec

All endpoint metadata comes from the Onshape OpenAPI specification (`crates/onshape-mcp-io/onshape-openapi.json`). This makes the tools self-documenting and automatically up-to-date.

| Setting | Value |
| --------- | ------- |
| Location | `crates/onshape-mcp-io/onshape-openapi.json` |
| Source | `https://cad.onshape.com/api/v6/openapi` (spec download URL; `v6` is the download endpoint version) |
| Server URL | Parsed from `servers[0].url` in the spec (currently `v14`, the API base-path version) |
| License | Apache 2.0 (see `crates/onshape-mcp-io/ONSHAPE-API-LICENSE`) |
| Loading | Embedded at compile time via `include_str!()` |

## Tool Parameters

### Identifiers

Onshape uses compound identifiers. When calling API endpoints, these are passed as separate parameters:

- `did` — Document ID
- `wid` — Workspace ID
- `vid` — Version ID
- `eid` — Element ID
