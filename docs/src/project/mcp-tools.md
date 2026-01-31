# MCP Tools

## Primary Use Cases

1. **Exporting** — Get data out of Onshape (STL, STEP, glTF, etc.)
2. **Exploring** — Navigate and understand existing designs
3. **AI-assisted FeatureScript development** — Later phase

## Target Users

Individuals first, with architecture that doesn't preclude teams.

## Tool Naming Convention

All MCP tools use the `onshape_` prefix to avoid collisions with other MCP servers.

| Prefix | Purpose | Example |
| -------- | --------- | --------- |
| `onshape_api_` | Onshape REST API operations | `onshape_api_list_documents` |
| `onshape_mcp_` | MCP server administration | `onshape_mcp_get_mode` |

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
| `read` | Read-only tools | Query, list, export |
| `modify` | Read + non-destructive writes | Add, update, set |
| `destroy` | All tools | Delete, remove |

### Mode Configuration

Mode settings are configured via the standard configuration system. See [Configuration](configuration.md#all-settings-reference) for details on `max_mode`, `initial_mode`, and `allow_mode_escalation`.

**Why explicit `allow_mode_escalation`?** Without it, we cannot distinguish between:

- User wants AI to escalate when needed (interactive)
- User set max_mode as ceiling but controls mode manually per session

### Tool Visibility by Mode

Tools are hidden (not advertised) when the current mode doesn't permit them. This is cleaner than advertising tools that will be rejected.

| Tool | Required Mode | `readOnlyHint` | `destructiveHint` |
| ------ | --------------- | ---------------- | ------------------- |
| `onshape_api_list_documents` | `read` | `true` | — |
| `onshape_api_get_assembly` | `read` | `true` | — |
| `onshape_api_export_stl` | `read` | `true` | — |
| `onshape_api_set_variable` | `modify` | `false` | `false` |
| `onshape_api_add_feature` | `modify` | `false` | `false` |
| `onshape_api_update_feature` | `modify` | `false` | `false` |
| `onshape_api_delete_feature` | `destroy` | `false` | `true` |

### MCP Tool Annotations

Tools declare their characteristics using MCP's `ToolAnnotations`:

- `readOnlyHint` — true if tool doesn't modify Onshape data
- `destructiveHint` — true if tool performs destructive operations

These are advisory hints for MCP clients, not security enforcement.

## Server Administration Tools

Always visible (read-only operations on the server itself).

| Tool | Description |
| ------ | ------------- |
| `onshape_mcp_get_mode` | Returns current mode, max mode, escalation allowed |
| `onshape_mcp_request_mode` | Request mode change (escalate or de-escalate, within max) |
| `onshape_mcp_auth_status` | Returns auth status (valid/invalid/expired), last check time, connectivity |

## Onshape API Tools

### Phase A: Read-Only Foundation (MVP)

| Tool | Mode | Description |
| ------ | ------ | ------------- |
| **Documents** | | |
| `onshape_api_list_documents` | `read` | List user's documents (with search/filter) |
| `onshape_api_get_document` | `read` | Get document metadata, workspaces, versions |
| `onshape_api_list_elements` | `read` | List elements (tabs) in a document |
| **Part Studios** | | |
| `onshape_api_get_part_studio` | `read` | Get part studio metadata |
| `onshape_api_list_features` | `read` | List features in a part studio |
| `onshape_api_get_feature` | `read` | Get details of a specific feature |
| `onshape_api_list_parts` | `read` | List parts in a part studio |
| `onshape_api_get_mass_properties` | `read` | Get mass, volume, center of mass |
| `onshape_api_get_bounding_box` | `read` | Get bounding box |
| **Assemblies** | | |
| `onshape_api_get_assembly` | `read` | Get assembly definition |
| `onshape_api_get_bom` | `read` | Get bill of materials |
| `onshape_api_list_instances` | `read` | List parts/subassemblies |
| **Variables & Configurations** | | |
| `onshape_api_list_variables` | `read` | List variables in a part studio |
| `onshape_api_list_configurations` | `read` | List configuration options |

### Phase B: Export (MVP)

Export tools pass through to Onshape's export API. Tool names mirror Onshape's format names. All formats supported by Onshape are supported — examples include:

| Tool | Mode | Description |
| ------ | ------ | ------------- |
| `onshape_api_export_stl` | `read` | Export part/assembly as STL |
| `onshape_api_export_step` | `read` | Export as STEP |
| `onshape_api_export_gltf` | `read` | Export as glTF |
| `onshape_api_export_parasolid` | `read` | Export as Parasolid |
| `onshape_api_export_iges` | `read` | Export as IGES |
| `onshape_api_export_drawing_pdf` | `read` | Export drawing as PDF |
| ... | `read` | Other formats as provided by Onshape API |

#### Export Destination

Exports support two modes: returning a download URL (default) or saving to a local file.

**Parameters:**

| Parameter | Type | Default | Description |
| ----------- | ------ | --------- | ------------- |
| `save_to` | `string?` | `null` | Local file path. If omitted, returns URL only. |
| `overwrite` | `bool` | `false` | If `false`, fail when file exists. |

**Return Value (URL mode):**

```json
{
  "url": "https://...",
  "expires_at": "2024-01-15T10:30:00Z",
  "expires_in_seconds": 300
}
```

**Return Value (local file mode):**

```json
{
  "path": "/path/to/file.stl",
  "size_bytes": 12345
}
```

**Error Types (local file mode):**

| Error | Description |
| ------- | ------------- |
| `file_exists` | File exists and `overwrite=false` |
| `permission_denied` | Cannot write to path |
| `out_of_space` | Insufficient disk space |
| `invalid_path` | Path invalid or directory doesn't exist |
| `download_failed` | Failed to download from Onshape |

**Tool Description Note:** Export tools will document that download URLs are temporary (typically 5-15 minutes; exact expiration in response if available from API). Use `save_to` to download immediately if the file will be needed later.

### Phase C: Modify Operations

| Tool | Mode | Description |
| ------ | ------ | ------------- |
| `onshape_api_set_variable` | `modify` | Update a variable value |
| `onshape_api_set_configuration` | `modify` | Set configuration values |
| `onshape_api_add_feature` | `modify` | Add a feature to a part studio |
| `onshape_api_update_feature` | `modify` | Modify an existing feature |

### Phase D: Destroy Operations

| Tool | Mode | Description |
| ------ | ------ | ------------- |
| `onshape_api_delete_feature` | `destroy` | Remove a feature |

### Phase E: FeatureScript (Future)

| Tool | Mode | Description |
| ------ | ------ | ------------- |
| `onshape_api_eval_featurescript` | `destroy` | Execute FeatureScript expressions |
| `onshape_api_get_featurescript_spec` | `read` | Get FeatureScript function specs |
| `onshape_api_list_custom_features` | `read` | List available custom features |

## Tool Parameters

### Pagination

Tools that return lists expose pagination parameters:

- `limit` — Maximum items to return
- `offset` — Starting offset

### Identifiers

Onshape uses compound identifiers. Tools accept these as separate parameters:

- `document_id`
- `workspace_id` or `version_id`
- `element_id`
