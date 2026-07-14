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
| `onshape_mcp_` | MCP server administration | `onshape_mcp_get_started` |
| `onshape_` | Higher-level convenience tools | `onshape_screenshot` |

## Transport Support

| Transport | Priority | Notes |
| ----------- | ---------- | ------- |
| stdio | P0 | Primary MCP transport |
| Streamable HTTP | Experimental | Available for self-hosting; not broadly verified; no publicly offered endpoint; ChatGPT failure tracked in [#546](https://github.com/altendky/onshape-mcp/issues/546) |
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
| `onshape_auth_status` | true | false |
| `onshape_auth_login` | false | false |
| `onshape_api_search` | true | false |
| `onshape_api_explain` | true | false |
| `onshape_api_schema` | true | false |
| `onshape_api_call` | false | true |
| `onshape_screenshot` | true | false |

## Server Administration Tools

Always visible (read-only operations on the server itself).

| Tool | Description | Status |
| ------ | ------------- | ------ |
| `onshape_mcp_get_mode` | Returns current mode, max mode, escalation allowed | Not yet implemented |
| `onshape_mcp_request_mode` | Request mode change (escalate or de-escalate, within max) | Not yet implemented |
| `onshape_auth_status` | Returns auth status (valid/invalid/expired), last check time, connectivity | Implemented |
| `onshape_auth_login` | Start an OAuth authorization flow (proxy or direct mode) | Implemented |

### `onshape_auth_login`

Start an OAuth authorization flow. Returns a URL to open in your browser. After authorizing, the server automatically detects the new tokens.

**Input:**

| Parameter | Type | Required | Description |
| ----------- | -------- | ---------- | ------------- |
| `mode` | `string` | No | Login mode: `"direct"` (default) or `"proxy"` |
| `proxy_url` | `string` | Proxy only | Explicit nonblank URL of a self-hosted OAuth proxy |
| `client_id` | `string` | No | OAuth 2.0 client ID (required for direct mode) |
| `client_secret` | `string` | No | OAuth 2.0 client secret (required for direct mode) |

**Output:** A text message containing the authorization URL to open in the browser.

**Effect Pattern:** This tool uses the effects-as-data pattern. The core crate validates inputs and returns an `OAuthLoginFlow` effect with the login mode. The I/O layer starts a local callback server on `127.0.0.1:18338`, generates PKCE and CSRF tokens, builds the authorization URL, and orchestrates the code exchange and token persistence in the background.

**Modes:**

- **Direct mode** (default): Requires `client_id` and `client_secret` from the user's Onshape OAuth application. Exchanges the authorization code directly with Onshape's token endpoint.
- **Proxy mode**: Requires an explicit self-hosted proxy URL. Fetches the `client_id` from its `/config` endpoint and delegates token exchange. No public proxy is provided.

The tool prevents concurrent login attempts — only one flow can be active at a
time. It is intended for local stdio use. HTTP clients authenticate while
connecting to the remote server instead. Direct secret arguments remain in the
tool interface pending [#548](https://github.com/altendky/onshape-mcp/issues/548),
so MCP clients should not retain them in transcripts or logs.

## Onshape API Tools

Instead of individual tools for each API endpoint (which would consume too much LLM context), the server provides three generic tools powered by the Onshape OpenAPI specification.

### Design Rationale

MCP clients enumerate all tools in the system prompt. With 30+ individual tools, the schema definitions alone would consume significant context. Three generic tools keep the footprint minimal while providing access to the full Onshape API.

### Tool Overview

| Tool | Purpose |
| ------ | --------- |
| `onshape_api_search` | Find Onshape API endpoints by keyword or filter |
| `onshape_api_explain` | Get full details for a specific endpoint |
| `onshape_api_schema` | Look up a component schema by name (BTType exploration) |
| `onshape_api_call` | Invoke an Onshape API endpoint with structured parameters |

### Workflow

An LLM uses these tools in a natural progression:

1. **Search** to find relevant endpoints
2. **Explain** to learn the parameters for a specific endpoint
3. **Schema** (if needed) to explore polymorphic request body types
4. **Call** to execute it

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

### `onshape_api_schema`

Look up a component schema by name.
Returns the schema's merged properties (own + inherited from parent via `allOf`), discriminator subtypes if the schema is polymorphic, and parent type information.

This tool enables on-demand exploration of the Onshape BTType hierarchy.
When `onshape_api_explain` returns a request body schema with `x-bttype-options` annotations on polymorphic properties, use this tool to drill into specific types and learn their properties.

**Input:**

| Parameter | Type | Required | Description |
| ----------- | -------- | ---------- | ------------- |
| `schema` | `string` | Yes | Schema name (e.g., `"BTMParameterEnum-145"` or `"BTFeatureDefinitionCall-1406"`). Use names from `x-bttype-options` annotations or `subtypes` fields. |

**Output:** JSON object with schema detail:

```json
{
  "name": "BTMParameterEnum-145",
  "parent": "BTMParameter-1",
  "properties": {
    "btType": { "type": "string" },
    "parameterId": { "type": "string" },
    "parameterName": { "type": "string" },
    "enumName": { "type": "string" },
    "value": { "type": "string" }
  }
}
```

For polymorphic schemas, the output also includes `discriminator_property` and `subtypes`:

```json
{
  "name": "BTMParameter-1",
  "description": "A parameter value.",
  "properties": { "...": "..." },
  "discriminator_property": "btType",
  "subtypes": [
    "BTMParameterEnum-145",
    "BTMParameterQuantity-147",
    "BTMParameterString-149"
  ]
}
```

Fields `description`, `parent`, `required`, `subtypes`, and `discriminator_property` are omitted when null/empty.
Properties that are `$ref`s pointing to discriminator schemas include `x-bttype-options` annotations, enabling further drill-down.

#### Discriminator Annotations in `onshape_api_explain`

When `onshape_api_explain` returns a request body or response schema, properties that reference polymorphic types are annotated with `x-bttype-options`.
For example, a `feature` property referencing `BTMFeature-134` would appear as:

```json
{
  "feature": {
    "$ref": "#/components/schemas/BTMFeature-134",
    "x-bttype-options": [
      "BTMSketch-151",
      "BTMFeatureInvalid-1031"
    ]
  }
}
```

This tells the LLM which concrete types are valid without expanding the full schema.
Use `onshape_api_schema` to look up the details of whichever type is needed.

### `onshape_api_call`

Invoke an Onshape API endpoint with structured parameters. Path parameters are named fields, not baked into a URL string. The tool resolves the path template internally using the spec.

**Input:**

| Parameter | Type | Required | Description |
| ----------- | -------- | ---------- | ------------- |
| `endpoint` | `string` | Yes | The operation ID to call |
| `path_params` | `object` | No | Path parameters (e.g., `{"did": "abc123"}`) |
| `query_params` | `object` | No | Query parameters (e.g., `{"q": "robot arm", "limit": "10"}`) |
| `header_params` | `object` | No | Header parameters (e.g., `{"Accept": "application/octet-stream"}`) |
| `body` | `any` | No | Request body (for POST/PUT/PATCH endpoints) |
| `file_refs` | `array` | No | File content to inject into multipart or JSON request body fields |

**Output:** The API response content. JSON responses are returned as JSON content;
text responses are returned as text. Binary responses are returned as JSON metadata
with `encoding: "base64"`, `byteLength`, `contentType` when present, and the
base64-encoded `body`.

`header_params` can set request headers such as `Accept`. If no `Accept` is
provided, the server selects one from the endpoint's declared OpenAPI response
media types, preferring JSON when available and otherwise using the declared
binary/media type. Authentication remains executor-owned; caller-supplied
`Authorization` headers are ignored and replaced with the configured Onshape
credentials.

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

## Convenience Tools

Higher-level tools that wrap Onshape API endpoints with agent-friendly interfaces.

### `onshape_screenshot`

Take a screenshot of a Part Studio. Renders a single view server-side and saves the PNG to disk. Returns the file path (not image data), so the agent never needs to handle base64. Call multiple times for multiple views.

Wraps the `getPartStudioShadedViews` endpoint with these improvements:

- **Always uses `pixelSize=0`** (auto-fit) so parts fill the image
- **Angular view control** via named presets or azimuth/elevation instead of raw 3x4 matrices
- **Automatic base64 decode and file save**
- **Computed view matrix included in the response** for debugging

**Input:**

| Parameter | Type | Required | Description |
| --------- | ---- | -------- | ----------- |
| `did` | `string` | Yes | Document ID |
| `wvm` | `string` | Yes | `"w"`, `"v"`, or `"m"` |
| `wvmid` | `string` | Yes | Workspace/Version/Microversion ID |
| `eid` | `string` | Yes | Part Studio element ID |
| `view` | `object` | Yes | View specification (see below) |
| `output_path` | `string` | Yes | Full file path for the output PNG (e.g., `"/tmp/screenshot.png"`) |
| `output_height` | `integer` | No | Image height in pixels (default: 500) |
| `output_width` | `integer` | No | Image width in pixels (default: 500) |
| `edges` | `string` | No | `"show"` or `"hide"` (default: `"show"`) |
| `use_anti_aliasing` | `boolean` | No | Default: false |
| `show_all_parts` | `boolean` | No | Default: false |
| `include_surfaces` | `boolean` | No | Default: false |
| `include_wires` | `boolean` | No | Default: false |

**View Specification** (tagged enum, discriminated by `type`):

```json
{"type": "preset", "name": "front"}
{"type": "preset", "name": "isometric"}
{"type": "angles", "azimuth": 45, "elevation": 30}
```

Available presets: `front`, `back`, `top`, `bottom`, `left`, `right`, `isometric`.

For angles: `azimuth` is horizontal orbit in degrees (0=front, 90=right, 180=back, 270=left). `elevation` is vertical tilt above horizontal (-90 to 90).

**Output:** Two content blocks:

1. Structured JSON:

```json
{
  "path": "/tmp/screenshot.png",
  "view": "front",
  "view_matrix": "front",
  "status": "ok"
}
```

1. Human-readable summary with file path, view label, and computed view matrix.

**Effect Pattern:** Uses `OnshapeApiRequestThen` → `WriteFiles`. The core crate builds the API request, the I/O layer executes it, the core callback decodes the base64 image (pure computation) and returns a `WriteFiles` effect, and the I/O layer writes the file to disk. The core's format callback then produces the final result from the write outcome.

## Tool Parameters

### Identifiers

Onshape uses compound identifiers. When calling API endpoints, these are passed as separate parameters:

- `did` — Document ID
- `wid` — Workspace ID
- `vid` — Version ID
- `eid` — Element ID
