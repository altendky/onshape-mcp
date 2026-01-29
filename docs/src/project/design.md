# Onshape MCP - Project Requirements

## Overview

A Rust-based MCP (Model Context Protocol) server for Onshape integration. The project emphasizes testability through sans-IO design principles and comprehensive cross-platform support.

## Core Requirements

### Platform Support

| Platform | Architecture | Status   |
|----------|--------------|----------|
| Linux    | x86_64       | Required |
| Linux    | aarch64      | Required |
| macOS    | x86_64       | Required |
| macOS    | aarch64      | Required |
| Windows  | x86_64       | Required |
| Windows  | aarch64      | Required |

**Constraints:**

- No platform-specific code without abstraction
- No explicit constraints against supporting additional platforms
- All code must compile for all target platforms

### Technology Choices

| Component            | Choice                     | Rationale                                                                          |
|----------------------|----------------------------|------------------------------------------------------------------------------------|
| MCP SDK              | `rmcp` (official Rust SDK) | Official implementation, maintained, tokio-based                                   |
| Async Runtime        | Tokio                      | Required by rmcp, best ecosystem support                                           |
| Configuration        | `figment`                  | Layered config with excellent error provenance, first-class serde/clap integration |
| CLI                  | `clap`                     | Derive macros, env var support, shell completions, integrates with figment         |
| Minimum Rust Version | 1.75+                      | Stable async traits, impl Trait in traits                                          |
| License              | MIT OR Apache-2.0          | Standard dual license for Rust projects                                            |

### Toolchain

| File                  | Purpose                              |
|-----------------------|--------------------------------------|
| `rust-toolchain.toml` | Pin toolchain version and components |

**Configuration:**

- **Channel:** MSRV (`1.75`)
- **Components:** `rustfmt`, `clippy`, `llvm-tools-preview`

Pinning to MSRV ensures developers default to the minimum supported version.

### Architecture: Sans-IO Design

The project will follow **full sans-IO design principles** to maximize testability:

```text
┌─────────────────────────────────────────────────────────────────┐
│                        Application Layer                         │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                        onshape-mcp                           ││
│  │         (binary - wires everything together)                 ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│                        Integration Layer                         │
│  ┌──────────────────────┐  ┌──────────────────────────────────┐│
│  │   onshape-mcp-io     │  │      onshape-client-io           ││
│  │ (MCP transport glue) │  │   (HTTP client for Onshape)      ││
│  └──────────────────────┘  └──────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│                          Core Layer                              │
│  ┌──────────────────────┐  ┌──────────────────────────────────┐│
│  │   onshape-mcp-core   │  │       onshape-client-core        ││
│  │  (pure protocol &    │  │     (pure Onshape API logic,     ││
│  │   business logic,    │  │      request/response types,     ││
│  │   NO I/O)            │  │      NO I/O)                     ││
│  └──────────────────────┘  └──────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

#### Sans-IO Principles

1. **Core crates have zero I/O dependencies** - no tokio, no async, no network, no filesystem
2. **Core crates CAN have pure computation dependencies** - `serde`, `thiserror`, `anyhow`, etc. are allowed
3. **State machines over async/await in core** - pure functions that produce effects
4. **Effects are data** - I/O operations are represented as data structures
5. **I/O crates interpret effects** - thin wrappers that execute the effects
6. **100% testable without mocking** - core logic tested with deterministic inputs

#### Example Sans-IO Pattern

```rust
// Core crate - pure logic, no I/O
pub enum Effect {
    SendMcpResponse(Response),
    CallOnshapeApi(OnshapeRequest),
    Log(LogLevel, String),
}

pub struct McpHandler {
    state: State,
}

impl McpHandler {
    // Pure function: input -> (new_state, effects)
    pub fn handle_request(&mut self, request: Request) -> Vec<Effect> {
        // Pure logic here, returns effects to be executed
    }
}

// I/O crate - interprets effects
async fn run_effects(effects: Vec<Effect>, transport: &mut Transport) {
    for effect in effects {
        match effect {
            Effect::SendMcpResponse(r) => transport.send(r).await,
            // ...
        }
    }
}
```

### Dependencies Policy

#### Core Crates (sans-IO)

**Allowed:**

- `serde` (serialization)
- `schemars` (JSON schema generation)
- `thiserror` (error types)
- `anyhow` (internal error context)
- Pure computation crates

**Forbidden:**

- `tokio` or any async runtime
- `reqwest`, `hyper` or HTTP clients
- File system access
- Network access

#### I/O Crates

**Allowed:**

- `tokio`
- `rmcp`
- `reqwest`
- Transport-specific crates

## Project Structure

### Workspace Layout

```text
onshape-mcp/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── onshape-mcp-core/         # Pure MCP logic (sans-IO)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-mcp-io/           # MCP I/O layer (tokio, rmcp)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-client-core/      # Pure Onshape API logic (sans-IO)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-client-io/        # Onshape HTTP client (reqwest)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-mcp/              # Main binary
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── tracing-sansio/           # Sans-IO tracing capture
│   │   ├── Cargo.toml
│   │   └── src/
│   └── tracing-sansio-macros/    # Proc-macro for tracing-sansio
│       ├── Cargo.toml
│       └── src/
├── tests/                        # Integration tests
├── .github/
│   └── workflows/
│       ├── ci.yml
│       ├── rust.yml
│       └── update-openapi-spec.yml
├── rust-toolchain.toml
├── .pre-commit-config.yaml
├── typos.toml
├── specs/
│   ├── onshape-openapi.json
│   └── ONSHAPE-API-LICENSE
├── docs/
│   ├── book.toml
│   └── src/
│       ├── SUMMARY.md
│       └── project/
│           └── *.md
├── README.md
├── LICENSE-MIT
└── LICENSE-APACHE
```

## Configuration

### Architecture & Patterns

Configuration uses `figment` for layered configuration with `clap` for CLI argument parsing. This provides:

- Excellent error provenance (know exactly where a value came from)
- First-class serde integration
- Multiple source support with clear precedence

### Configuration Precedence

From lowest to highest priority:

1. **Defaults** (hardcoded)
2. **Config file**
3. **Environment variables**
4. **CLI flags**

### Config File

| Platform | Location                              |
|----------|---------------------------------------|
| Unix     | `~/.config/onshape-mcp/config.toml`   |
| Windows  | `%APPDATA%\onshape-mcp\config.toml`   |

Example config file:

```toml
[auth]
access_key = "..."
secret_key = "..."
check_interval = "5m"

[mode]
max = "read"
initial = "read"
allow_escalation = false
```

### Environment Variables

All environment variables use the `ONSHAPE_MCP_` prefix.

### All Settings Reference

| Setting               | Type                      | Default | Env Var                           | Config Key              | Description                                   |
|-----------------------|---------------------------|---------|-----------------------------------|-------------------------|-----------------------------------------------|
| Access Key            | `string`                  | —       | `ONSHAPE_MCP_ACCESS_KEY`          | `auth.access_key`       | Onshape API access key                        |
| Secret Key            | `string`                  | —       | `ONSHAPE_MCP_SECRET_KEY`          | `auth.secret_key`       | Onshape API secret key                        |
| Max Mode              | `read`/`modify`/`destroy` | `read`  | `ONSHAPE_MCP_MAX_MODE`            | `mode.max`              | Upper limit for permission mode               |
| Initial Mode          | `read`/`modify`/`destroy` | `read`  | `ONSHAPE_MCP_INITIAL_MODE`        | `mode.initial`          | Starting permission mode (must be ≤ max_mode) |
| Allow Mode Escalation | `bool`                    | `false` | `ONSHAPE_MCP_ALLOW_ESCALATION`    | `mode.allow_escalation` | Can AI change mode at runtime?                |
| Auth Check Interval   | `duration`                | `5m`    | `ONSHAPE_MCP_AUTH_CHECK_INTERVAL` | `auth.check_interval`   | Periodic credential validation interval       |

## MCP Server Functionality

### Primary Use Cases

1. **Exporting** — Get data out of Onshape (STL, STEP, glTF, etc.)
2. **Exploring** — Navigate and understand existing designs
3. **AI-assisted FeatureScript development** — Later phase

### Target Users

Individuals first, with architecture that doesn't preclude teams.

### Tool Naming Convention

All MCP tools use the `onshape_` prefix to avoid collisions with other MCP servers.

| Prefix         | Purpose                  | Example                    |
|----------------|--------------------------|----------------------------|
| `onshape_`     | Onshape API operations   | `onshape_list_documents`   |
| `onshape_mcp_` | Server administration    | `onshape_mcp_get_mode`     |

**Build-time check:** A build-time test must verify that `onshape_mcp_` does not collide with any Onshape API endpoint names.

### Transport Support

| Transport | Priority | Notes                 |
|-----------|----------|-----------------------|
| stdio     | P0       | Primary MCP transport |
| HTTP/SSE  | P1       | Server-Sent Events    |
| WebSocket | P2       | Bidirectional         |

### Permission Model

The server supports three permission modes controlling which tools are visible and callable.

#### Modes

| Mode      | Tools Available               | Description         |
|-----------|-------------------------------|---------------------|
| `read`    | Read-only tools               | Query, list, export |
| `modify`  | Read + non-destructive writes | Add, update, set    |
| `destroy` | All tools                     | Delete, remove      |

#### Mode Configuration

Mode settings are configured via the standard configuration system. See [Configuration > All Settings Reference](#all-settings-reference) for details on `max_mode`, `initial_mode`, and `allow_mode_escalation`.

**Why explicit `allow_mode_escalation`?** Without it, we cannot distinguish between:

- User wants AI to escalate when needed (interactive)
- User set max_mode as ceiling but controls mode manually per session

#### Tool Visibility by Mode

Tools are hidden (not advertised) when the current mode doesn't permit them. This is cleaner than advertising tools that will be rejected.

| Tool                     | Required Mode | `readOnlyHint` | `destructiveHint` |
|--------------------------|---------------|----------------|-------------------|
| `onshape_list_documents` | `read`        | `true`         | —                 |
| `onshape_get_assembly`   | `read`        | `true`         | —                 |
| `onshape_export_stl`     | `read`        | `true`         | —                 |
| `onshape_set_variable`   | `modify`      | `false`        | `false`           |
| `onshape_add_feature`    | `modify`      | `false`        | `false`           |
| `onshape_update_feature` | `modify`      | `false`        | `false`           |
| `onshape_delete_feature` | `destroy`     | `false`        | `true`            |

#### MCP Tool Annotations

Tools declare their characteristics using MCP's `ToolAnnotations`:

- `readOnlyHint` — true if tool doesn't modify Onshape data
- `destructiveHint` — true if tool performs destructive operations

These are advisory hints for MCP clients, not security enforcement.

### MCP Tools — Server Administration

Always visible (read-only operations on the server itself).

| Tool                       | Description                                                                 |
|----------------------------|-----------------------------------------------------------------------------|
| `onshape_mcp_get_mode`     | Returns current mode, max mode, escalation allowed                          |
| `onshape_mcp_request_mode` | Request mode change (escalate or de-escalate, within max)                   |
| `onshape_mcp_auth_status`  | Returns auth status (valid/invalid/expired), last check time, connectivity  |

### MCP Tools — Onshape API

#### Phase A: Read-Only Foundation (MVP)

| Tool                             | Mode   | Description                                 |
|----------------------------------|--------|---------------------------------------------|
| **Documents**                    |        |                                             |
| `onshape_list_documents`         | `read` | List user's documents (with search/filter)  |
| `onshape_get_document`           | `read` | Get document metadata, workspaces, versions |
| `onshape_list_elements`          | `read` | List elements (tabs) in a document          |
| **Part Studios**                 |        |                                             |
| `onshape_get_part_studio`        | `read` | Get part studio metadata                    |
| `onshape_list_features`          | `read` | List features in a part studio              |
| `onshape_get_feature`            | `read` | Get details of a specific feature           |
| `onshape_list_parts`             | `read` | List parts in a part studio                 |
| `onshape_get_mass_properties`    | `read` | Get mass, volume, center of mass            |
| `onshape_get_bounding_box`       | `read` | Get bounding box                            |
| **Assemblies**                   |        |                                             |
| `onshape_get_assembly`           | `read` | Get assembly definition                     |
| `onshape_get_bom`                | `read` | Get bill of materials                       |
| `onshape_list_instances`         | `read` | List parts/subassemblies                    |
| **Variables & Configurations**   |        |                                             |
| `onshape_list_variables`         | `read` | List variables in a part studio             |
| `onshape_list_configurations`    | `read` | List configuration options                  |

#### Phase B: Export (MVP)

Export tools pass through to Onshape's export API. Tool names mirror Onshape's format names. All formats supported by Onshape are supported — examples include:

| Tool                         | Mode   | Description                              |
|------------------------------|--------|------------------------------------------|
| `onshape_export_stl`         | `read` | Export part/assembly as STL              |
| `onshape_export_step`        | `read` | Export as STEP                           |
| `onshape_export_gltf`        | `read` | Export as glTF                           |
| `onshape_export_parasolid`   | `read` | Export as Parasolid                      |
| `onshape_export_iges`        | `read` | Export as IGES                           |
| `onshape_export_drawing_pdf` | `read` | Export drawing as PDF                    |
| ...                          | `read` | Other formats as provided by Onshape API |

#### Export Destination

Exports support two modes: returning a download URL (default) or saving to a local file.

**Parameters:**

| Parameter   | Type      | Default | Description                                    |
|-------------|-----------|---------|------------------------------------------------|
| `save_to`   | `string?` | `null`  | Local file path. If omitted, returns URL only. |
| `overwrite` | `bool`    | `false` | If `false`, fail when file exists.             |

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

| Error               | Description                              |
|---------------------|------------------------------------------|
| `file_exists`       | File exists and `overwrite=false`        |
| `permission_denied` | Cannot write to path                     |
| `out_of_space`      | Insufficient disk space                  |
| `invalid_path`      | Path invalid or directory doesn't exist  |
| `download_failed`   | Failed to download from Onshape          |

**Tool Description Note:** Export tools will document that download URLs are temporary (typically 5-15 minutes; exact expiration in response if available from API). Use `save_to` to download immediately if the file will be needed later.

#### Phase C: Modify Operations

| Tool                        | Mode     | Description                     |
|-----------------------------|----------|---------------------------------|
| `onshape_set_variable`      | `modify` | Update a variable value         |
| `onshape_set_configuration` | `modify` | Set configuration values        |
| `onshape_add_feature`       | `modify` | Add a feature to a part studio  |
| `onshape_update_feature`    | `modify` | Modify an existing feature      |

#### Phase D: Destroy Operations

| Tool                     | Mode      | Description      |
|--------------------------|-----------|------------------|
| `onshape_delete_feature` | `destroy` | Remove a feature |

#### Phase E: FeatureScript (Future)

| Tool                             | Mode      | Description                        |
|----------------------------------|-----------|------------------------------------|
| `onshape_eval_featurescript`     | `destroy` | Execute FeatureScript expressions  |
| `onshape_get_featurescript_spec` | `read`    | Get FeatureScript function specs   |
| `onshape_list_custom_features`   | `read`    | List available custom features     |

### Tool Parameters

#### Pagination

Tools that return lists expose pagination parameters:

- `limit` — Maximum items to return
- `offset` — Starting offset

#### Identifiers

Onshape uses compound identifiers. Tools accept these as separate parameters:

- `document_id`
- `workspace_id` or `version_id`
- `element_id`

## Authentication

### Supported Methods

| Method    | Status                 | Notes                        |
|-----------|------------------------|------------------------------|
| API Keys  | Initial implementation | Personal use, single user    |
| OAuth 2.0 | Future                 | Multi-user apps, team access |

### Credential Sources

Credentials can be provided via config file or environment variables. See [Configuration](#configuration) for file locations and precedence rules.

**Config file example:**

```toml
[auth]
access_key = "..."
secret_key = "..."
```

**Environment variables:**

| Variable                 | Description              |
|--------------------------|--------------------------|
| `ONSHAPE_MCP_ACCESS_KEY` | Onshape API access key   |
| `ONSHAPE_MCP_SECRET_KEY` | Onshape API secret key   |

**Future credential sources** (to be implemented):

- System keychain integration

### Config File Security

The config file contains secrets and must have restricted permissions:

- **Unix:** `0600` (owner read/write only)
- **Windows:** Equivalent ACL restrictions

If permissions are too open, the server **blocks access** and informs the user of the issue.

### Credential Validation

| Event               | Behavior                                                                                                     |
|---------------------|--------------------------------------------------------------------------------------------------------------|
| Startup             | Validate credentials, fail if invalid                                                                        |
| Periodic            | Re-validate at configured interval (see `auth.check_interval` in [Configuration](#all-settings-reference))   |
| API call            | Updates auth status, resets periodic check timer                                                             |
| Invalid credentials | Fail API calls with clear error, emit MCP notification                                                       |

**No caching:** If credentials become invalid mid-session, all subsequent API calls fail until credentials are fixed.

### MCP Notifications

The server emits MCP notifications for auth status changes:

- `onshape/auth/invalid` — Credentials became invalid
- `onshape/auth/restored` — Credentials are valid again

## Error Handling

### Strategy

Use `thiserror` for public API errors (typed, matchable) with `anyhow` for internal convenience and context chaining.

### Onshape API Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum OnshapeApiError {
    #[error("Authentication failed")]
    AuthenticationFailed,

    #[error("Permission denied: {message}")]
    PermissionDenied { message: String },

    #[error("Not found: {resource}")]
    NotFound { resource: String },

    #[error("Rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },

    #[error("Request timeout")]
    Timeout,

    #[error("Server error: {status}")]
    ServerError { status: u16, message: Option<String> },

    #[error("CAD operation failed: {code}")]
    CadOperationFailed { code: String, message: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

### HTTP Status Code Mapping

| HTTP Code | Error Type             |
|-----------|------------------------|
| 401       | `AuthenticationFailed` |
| 403       | `PermissionDenied`     |
| 404       | `NotFound`             |
| 408       | `Timeout`              |
| 429       | `RateLimited`          |
| 500, 503  | `ServerError`          |

### Error Context

All errors include context for debugging:

| Field        | Description                              |
|--------------|------------------------------------------|
| `request_id` | Onshape request ID from response headers |
| `endpoint`   | API endpoint called                      |
| `timestamp`  | When the error occurred                  |

### MCP Error Mapping

Hybrid approach: standard JSON-RPC codes where they fit, custom codes for actionable distinctions.

| Error Type             | MCP Code | Rationale               |
|------------------------|----------|-------------------------|
| `AuthenticationFailed` | `-32603` | Internal error          |
| `PermissionDenied`     | `-32603` | Internal error          |
| `NotFound`             | `-32602` | Invalid params (bad ID) |
| `RateLimited`          | `-32000` | Custom: retryable       |
| `Timeout`              | `-32000` | Custom: retryable       |
| `ServerError`          | `-32603` | Internal error          |
| `CadOperationFailed`   | `-32603` | Internal error          |

All MCP errors include structured `data` field:

```json
{
  "code": -32000,
  "message": "Rate limited, retry after 30 seconds",
  "data": {
    "error_type": "rate_limited",
    "retry_after_seconds": 30,
    "request_id": "abc123"
  }
}
```

## Logging & Tracing

### Rationale

The standard `tracing` library relies on global/thread-local subscriber state, which conflicts with sans-IO principles. After researching the ecosystem:

| Crate             | Purpose                     | Sans-IO Compatible |
|-------------------|-----------------------------|--------------------|
| `tracing`         | Core instrumentation macros | No (global state)  |
| `tracing-tunnel`  | Serializable event capture  | Yes                |
| `tracing-capture` | Testing/inspection          | Partial            |

`tracing-tunnel` provides the foundation:

- `TracingEventSender` - subscriber that serializes events to a callback
- `TracingEvent` - full span hierarchy (enter/exit/close, parent-child relationships)
- `TracingEventReceiver` - replays events to real subscribers

A helper crate `tracing-sansio` wraps this with minimal boilerplate.

### API Design

**Closure-based (like `Span::in_scope`):**

```rust
use tracing_sansio::{capture_tracing, Captured};

let Captured { result, events } = capture_tracing(|| {
    tracing::info_span!("operation").in_scope(|| {
        tracing::info!("processing");
        compute_something()
    })
});
```

**Attribute macro (like `#[tracing::instrument]`):**

```rust
#[capture_tracing]
fn process_data(data: &[u8]) -> usize {
    tracing::info!(len = data.len(), "processing");
    data.len()
}
// Returns: Captured<usize>
```

### Crate Structure

```text
crates/
├── tracing-sansio/           # Core library
│   ├── Cargo.toml
│   └── src/lib.rs
└── tracing-sansio-macros/    # Proc-macro (optional feature)
    ├── Cargo.toml
    └── src/lib.rs
```

| Crate                   | Purpose                                                                     |
|-------------------------|-----------------------------------------------------------------------------|
| `tracing-sansio`        | `Captured<T>` type, `capture_tracing()` function, re-exports `TracingEvent` |
| `tracing-sansio-macros` | `#[capture_tracing]` attribute macro                                        |

### Key Types

| Type           | Description                                                    |
|----------------|----------------------------------------------------------------|
| `Captured<T>`  | Wrapper containing `result: T` and `events: Vec<TracingEvent>` |
| `TracingEvent` | Re-exported from `tracing-tunnel`; represents spans and events |

### Dependencies

**`tracing-sansio`:**

- `tracing` ^0.1
- `tracing-tunnel` ^0.1 (with `sender` feature)
- `tracing-sansio-macros` (optional, default enabled)

**`tracing-sansio-macros`:**

- `syn` ^2, `quote` ^1, `proc-macro2` ^1

### Limitations

| Limitation                    | Workaround                                            |
|-------------------------------|-------------------------------------------------------|
| Sync functions only (macro)   | Use `capture_tracing()` closure for async             |
| Thread-local capture          | Events from spawned threads not captured              |
| Bounded channel (default 256) | Use `capture_tracing_with_capacity()` for high-volume |

### Integration Pattern

Core crates use standard `tracing` macros. Tests and I/O boundaries use `capture_tracing()`:

```rust
// Core crate - standard tracing
pub fn handle_request(&mut self, req: Request) -> Response {
    tracing::info!(?req, "handling");
    // pure logic
}

// Test - explicit capture
#[test]
fn test_logs_request() {
    let Captured { result, events } = capture_tracing(|| {
        handler.handle_request(req)
    });
    assert!(events.iter().any(|e| matches!(e, TracingEvent::Event { .. })));
}
```

### Future Enhancements

See [Deferred > tracing-sansio Enhancements](#deferred) for planned improvements.

## Repository & CI

### GitHub Repository Settings

This section documents the manual configuration required in GitHub repository settings.

#### Branch Protection (main)

| Setting                     | Value                               |
|-----------------------------|-------------------------------------|
| Require PR before merge     | Yes                                 |
| Required approvals          | 0 (increase when contributors join) |
| Require status checks       | Yes — `alls-green` job only         |
| Require merge queue         | Yes                                 |
| Require branches up-to-date | No (merge queue handles this)       |
| Show update branch button   | Always                              |
| Require signed commits      | Yes                                 |
| Include administrators      | Yes                                 |

#### Merge Queue

Merge queue enabled to guarantee main stays green. PRs merge only after passing CI on the queued merge commit. This replaces the need for "require branches to be up-to-date" which can cause CI thrashing.

#### Merge Strategy

| Option        | Enabled    |
|---------------|------------|
| Merge commits | Yes (only) |
| Squash merge  | No         |
| Rebase merge  | No         |

#### Other Settings

| Setting                   | Value                                                                       |
|---------------------------|-----------------------------------------------------------------------------|
| Description               | "A Rust-based MCP (Model Context Protocol) server for Onshape integration." |
| Default branch            | `main`                                                                      |
| Auto-delete head branches | Yes                                                                         |
| Discussions               | Disabled                                                                    |
| Wiki                      | Disabled                                                                    |
| Projects                  | Disabled                                                                    |
| Issues                    | Enabled                                                                     |
| Pull Requests             | Enabled                                                                     |

#### GitHub App

Create a GitHub App for CI to run on auto-generated PRs (e.g., OpenAPI spec updates):

| Setting      | Value                                     |
|--------------|-------------------------------------------|
| Permissions  | `contents: write`, `pull-requests: write` |
| Installation | Repository only                           |
| Webhook      | Disabled (not needed)                     |

Store credentials in repository secrets:

- `APP_ID` — from app settings page
- `APP_PRIVATE_KEY` — contents of generated `.pem` file

### Workflow Structure

| File                                        | Purpose                                                   |
|---------------------------------------------|-----------------------------------------------------------|
| `.github/workflows/ci.yml`                  | Entry point, Rust version matrix, alls-green aggregation  |
| `.github/workflows/rust.yml`                | Reusable workflow, platform matrix, all checks            |
| `.github/workflows/update-openapi-spec.yml` | Nightly/manual OpenAPI spec update, creates PR            |

### CI Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│                    ci.yml (entry point)                      │
├─────────────────────────────────────────────────────────────┤
│  matrix:                                                     │
│    rust: [1.75, stable, beta]                               │
│                                                              │
│  jobs:                                                       │
│    build:                                                    │
│      uses: ./.github/workflows/rust.yml                     │
│      with:                                                   │
│        rust-version: ${{ matrix.rust }}                     │
│                                                              │
│    alls-green:                                               │
│      needs: [build]                                          │
│      allowed-failures: [beta]                                │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│              rust.yml (reusable workflow)                    │
├─────────────────────────────────────────────────────────────┤
│  inputs:                                                     │
│    rust-version: (required)                                  │
│                                                              │
│  matrix (internal):                                          │
│    os: [ubuntu, macos, windows]                             │
│    arch: [x86_64, aarch64]                                  │
│                                                              │
│  jobs (all 6 platform combinations):                         │
│    - setup-rust-toolchain (with rust-version)               │
│    - cargo fmt --check                                       │
│    - cargo clippy                                            │
│    - cargo test                                              │
│    - cargo deny                                              │
│                                                              │
│  jobs (stable × all 6 platforms):                            │
│    - coverage (cargo-llvm-cov)                              │
└─────────────────────────────────────────────────────────────┘
```

### Version & Platform Matrices

#### Rust Version Matrix

| Toolchain     | Required | Notes                        |
|---------------|----------|------------------------------|
| MSRV (1.75)   | Yes      | From `rust-toolchain.toml`   |
| Latest stable | Yes      | Primary development target   |
| Beta          | No       | Allowed to fail              |

#### Platform Matrix

| OS             | Architecture    |
|----------------|-----------------|
| Linux (ubuntu) | x86_64, aarch64 |
| macOS          | x86_64, aarch64 |
| Windows        | x86_64, aarch64 |

**Total jobs:**

- Checks: 3 rust × 6 platforms = 18 jobs
- Coverage: 1 rust (stable) × 6 platforms = 6 jobs
- **Total: 24 jobs** (plus alls-green)

### CI Tooling

| Tool                                                                                                | Purpose                                                  |
|-----------------------------------------------------------------------------------------------------|----------------------------------------------------------|
| [actions-rust-lang/setup-rust-toolchain](https://github.com/actions-rust-lang/setup-rust-toolchain) | Rust installation, reads `rust-toolchain.toml`, caching  |
| [re-actors/alls-green](https://github.com/re-actors/alls-green)                                     | Aggregate job status, allow beta failures                |

**GitHub branch protection:** Only the `alls-green` job is required.

### Checks

| Check            | Tool                | Run On                      |
|------------------|---------------------|-----------------------------|
| Formatting       | `cargo fmt --check` | All matrix combinations     |
| Linting          | `cargo clippy`      | All matrix combinations     |
| Tests            | `cargo test`        | All matrix combinations     |
| Dependency audit | `cargo deny`        | All matrix combinations     |
| Coverage         | `cargo-llvm-cov`    | Stable only, all platforms  |

### PR Title Enforcement

PR titles will be validated in CI. See [Deferred > CI/Infrastructure](#deferred) for details.

### Linting & Formatting

#### Linting Configuration

```toml
# clippy.toml or in CI
[lints.clippy]
pedantic = "warn"
nursery = "warn"
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
# Allow in tests
```

**Clippy Lints to Enable:**

- `clippy::pedantic` - additional strictness
- `clippy::nursery` - experimental but useful
- `clippy::unwrap_used` - deny in library code
- `clippy::expect_used` - deny in library code
- `clippy::panic` - deny in library code

#### Formatting

- Use `rustfmt` with default settings (or minimal customization)
- Enforce via CI

### Testing Strategy

| Test Type         | Location               | Coverage Target      |
|-------------------|------------------------|----------------------|
| Unit tests        | `crates/*/src/**/*.rs` | 100% with exclusions |
| Integration tests | `tests/`               | Key workflows        |
| Doc tests         | Inline                 | All public APIs      |

### Coverage Requirements

- **Tool:** `cargo-llvm-cov`
- **Reporting:** Codecov

#### Philosophy

**Target 100% coverage** with explicit exclusions for untestable code. The sans-IO architecture makes this achievable for core crates. Per-crate targets may be adjusted if specific crates prove less testable.

#### Enforcement Strategy

| Check            | Behavior                                                                               |
|------------------|----------------------------------------------------------------------------------------|
| Project coverage | Ratchet — fail if drops more than 2% from main (catches accidental loss of code/tests) |
| Patch coverage   | 100% enforced — new code must be fully covered or explicitly excluded                  |

Codecov configuration:

```yaml
coverage:
  status:
    project:
      default:
        threshold: 2%  # Catch accidental loss of code/tests
    patch:
      default:
        target: 100%  # Enforced; use LCOV exclusions for untestable code
```

#### Coverage Exclusions

Use LCOV comments to exclude code from coverage **with justification**:

```rust
// Platform-specific debug output not testable in CI
println!("debug: {}", value); // LCOV_EXCL_LINE

// LCOV_EXCL_START — Unreachable error path; requires external service failure
Err(e) => {
    log::error!("Unexpected: {}", e);
    panic!("invariant violated");
}
// LCOV_EXCL_STOP
```

**Policy:** Every exclusion must have a comment explaining why the code cannot or should not be tested. Enforced via code review.

### OpenAPI Spec Management

The Onshape OpenAPI specification is stored locally for reference and code generation.

| Setting  | Value                                        |
|----------|----------------------------------------------|
| Location | `specs/onshape-openapi.json`                 |
| Source   | `https://cad.onshape.com/api/v6/openapi`     |
| License  | Apache 2.0 (see `specs/ONSHAPE-API-LICENSE`) |
| Format   | Pretty-printed JSON                          |

#### Update Workflow

| Trigger | Schedule          |
|---------|-------------------|
| Nightly | 09:00 UTC         |
| Manual  | workflow_dispatch |

**Tooling:**

- [peter-evans/create-pull-request](https://github.com/peter-evans/create-pull-request) — Creates/updates PR when spec changes
- GitHub App token — Enables CI to run on auto-generated PRs

**Behavior:**

- Downloads latest spec from Onshape API
- Pretty-prints JSON for readable diffs
- Creates PR if changes detected (no PR on empty diff)
- Auto-merge enabled (optional, requires branch protection)
- Branch `automated/update-openapi-spec` deleted after merge

## Development Workflow

### Local Development

```bash
# Format
cargo fmt

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Test
cargo test

# Coverage
cargo llvm-cov --all-features --workspace
```

### Pre-commit Hooks

**Philosophy:** Pre-commit hooks provide developers an opt-in mechanism for fast local feedback. They do **not** enforce policy — CI is the source of truth.

**Tool:** [pre-commit](https://pre-commit.com/)

#### Hook Configuration

| Hook                   | Source           | Stage      | Purpose                |
|------------------------|------------------|------------|------------------------|
| `trailing-whitespace`  | pre-commit-hooks | pre-commit | Clean whitespace       |
| `end-of-file-fixer`    | pre-commit-hooks | pre-commit | Consistent EOF         |
| `check-toml`           | pre-commit-hooks | pre-commit | TOML syntax            |
| `check-yaml`           | pre-commit-hooks | pre-commit | YAML syntax            |
| `check-merge-conflict` | pre-commit-hooks | pre-commit | Catch conflict markers |
| `typos`                | crate-ci/typos   | pre-commit | Spell checking         |
| `cargo fmt --check`    | local            | pre-commit | Formatting             |
| `cargo clippy`         | local            | pre-commit | Linting                |
| `cargo test`           | local            | manual     | Tests                  |
| `cargo deny`           | local            | manual     | Dependency audit       |

**Stages:**

- `pre-commit` — runs automatically on `git commit`
- `manual` — runs only via `pre-commit run --hook-stage manual` or in CI

#### Configuration Files

| File                      | Purpose                    |
|---------------------------|----------------------------|
| `.pre-commit-config.yaml` | Hook definitions           |
| `typos.toml`              | Spell check word allowlist |

## Documentation

### Tooling

| Tool    | Purpose                                    |
|---------|--------------------------------------------|
| rustdoc | API documentation from source              |
| mdBook  | Prose documentation (design docs, guides)  |

### Structure

```text
docs/
├── book.toml
└── src/
    ├── SUMMARY.md
    ├── project/
    │   ├── principles.md      # Core philosophy (sans-IO, etc.)
    │   ├── architecture.md    # High-level design
    │   └── implementation.md  # Implementation plan/roadmap
    └── (future: user-guide/, etc.)
```

### Standards

*Still to discuss — see [Pending](#pending):*

- Rustdoc coverage expectations
- README content and structure
- Usage examples

## Distribution

Users can install the server via:

| Method             | Description                                  |
|--------------------|----------------------------------------------|
| `cargo install`    | From crates.io                               |
| Pre-built binaries | GitHub releases for all supported platforms  |

*Details to be discussed — see [Pending](#pending).*

## Release Process

*To be discussed — see [Pending](#pending).*

Areas to address:

- Versioning strategy (semver)
- Changelog management
- Release workflow (tags, GitHub releases)
- crates.io publishing

## Contribution Guidelines

*To be discussed — see [Pending](#pending).*

Areas to address:

- CONTRIBUTING.md content
- PR expectations
- Code review process
- Issue templates

## Open Questions

### Resolved

- [x] What Onshape API functionality should be exposed initially? — See MCP Server Functionality section
- [x] Configuration precedence? — Defaults → Config file → Env vars → CLI flags
- [x] Authentication strategy — API Keys initially, OAuth future; config file with permission checks; validation on startup + periodic
- [x] Error handling strategy — `thiserror` for public API, `anyhow` for internal; hybrid MCP error codes
- [x] Configuration tooling — `figment` for layered config (defaults → file → env → CLI)
- [x] GitHub App token — Use GitHub App with `actions/create-github-app-token@v1`; detailed setup during repository configuration
- [x] Coverage enforcement — 100% target with LCOV exclusions; 2% ratchet for project, 100% enforced for patches; see Coverage Requirements section
- [x] Export destination — URL default with optional `save_to` path; `overwrite` parameter; typed errors; see Phase B Export section
- [x] CLI library — `clap` with derive macros; integrates with figment via `Serialized::defaults(Args::parse())`
- [x] Repository setup — Branch protection, merge queue, signed commits, merge commits only; see Repository Setup section

### Pending

- [ ] Documentation standards — Rustdoc coverage, README structure, usage examples
- [ ] Distribution details — cargo install setup, GitHub releases workflow, binary naming
- [ ] npm wrapper package — Evaluate publishing JS wrapper for `npx` installation; check availability of `onshape-mcp`, consider scoped name or platform-specific optional deps pattern (like `swc`, `esbuild`)
- [ ] Release process — Versioning strategy, changelog, release workflow, crates.io publishing
- [ ] Contribution guidelines — CONTRIBUTING.md, PR expectations, code review process
- [ ] Project files — Standard files not yet documented (.gitignore, .gitattributes, rustfmt.toml, clippy.toml, deny.toml, .editorconfig, codecov.yml, CHANGELOG.md, dependabot.yml, CODEOWNERS, issue/PR templates)
- [ ] Non-manual repository configuration — Discuss opportunities for automated repo config (Terraform, GitHub API, etc.)
- [ ] Document separation — Discuss splitting into REQUIREMENTS.md, ARCHITECTURE.md, CONTRIBUTING.md
- [ ] Open Questions structure — Discuss whether Resolved items should remain as decision log, naming conventions
- [ ] Documentation review commands — Discuss commands/prompts for reviewing and managing documentation
- [ ] Implementation checklist location — Discuss moving checklist to separate document (e.g., `.opencode/plans/IMPLEMENTATION.md`)
- [ ] AI awareness setup — Create root-level AI context file (e.g., `AGENTS.md` or `CLAUDE.md`) that references REQUIREMENTS.md for project standards, architecture, and conventions; use symlinks for tool-specific locations (`.cursorrules`, `.github/copilot-instructions.md`, etc.) to maintain single source of truth; prioritize portable/standard formats over tool-specific syntax where possible
- [ ] AI file validation and testing — Review mechanisms for validating AI context files in CI (e.g., symlink integrity, reference validity, format linting)
- [ ] Git ignore strategy — Discuss root .gitignore vs distributed approach, required patterns (target/, IDE files, OS files, etc.)
- [ ] Markdown validation — Evaluate CI/pre-commit checks for markdown files (markdownlint for style/formatting, markdown-link-check for broken links, prose linting with vale)

### Deferred

Items to address later in the project:

#### CI/Infrastructure

- [ ] PR title enforcement — CI validation of PR title format (Conventional Commits or custom)
- [ ] Labels configuration — Standard labels for issues/PRs (bug, enhancement, etc.)

#### Authentication Enhancements

- [ ] OAuth 2.0 authentication — Multi-user apps, team access (see Authentication section)
- [ ] System keychain credential source — Platform-native secure storage

#### Features

- [ ] FeatureScript support — Phase E tools (`onshape_eval_featurescript`, etc.)

#### tracing-sansio Enhancements

- [ ] Independent publication — Extract to separate repository and publish to crates.io
- [ ] Async support — Add `capture_tracing_async()` if needed
- [ ] Event filtering — Add predicates to filter captured events

#### Markdown Tooling

- [ ] Semantic newlines enforcement — Add `markdownlint-sentences-per-line` rule to enforce one sentence per line (requires Node.js/npm infrastructure)

## Version History

| Version | Date | Changes              |
|---------|------|----------------------|
| 0.1.0   | TBD  | Initial requirements |

---

## Checklist for Implementation

### Phase 1: Project Setup

- [ ] Initialize Cargo workspace
- [ ] Set up crate structure
- [ ] Create rust-toolchain.toml
- [ ] Configure linting (clippy.toml, rustfmt.toml)
- [ ] Set up pre-commit hooks (.pre-commit-config.yaml, typos.toml)
- [ ] Set up GitHub Actions CI
- [ ] Set up specs/ directory with OpenAPI spec and license
- [ ] Create update-openapi-spec.yml workflow
- [ ] Configure GitHub App for CI triggering
- [ ] Add license files
- [ ] Create initial README

### Phase 1.5: Tracing Infrastructure

- [ ] Create `crates/tracing-sansio/Cargo.toml`
- [ ] Implement `Captured<T>` type and `capture_tracing()` function
- [ ] Create `crates/tracing-sansio-macros/Cargo.toml`
- [ ] Implement `#[capture_tracing]` proc-macro
- [ ] Add unit tests for `capture_tracing()`
- [ ] Add integration tests for `#[capture_tracing]` macro
- [ ] Add documentation with examples

### Phase 2: Core Implementation

- [ ] Define effect types in `onshape-mcp-core`
- [ ] Implement MCP handler state machine
- [ ] Implement permission model (modes, escalation)
- [ ] Define Onshape API types in `onshape-client-core`
- [ ] Implement MCP Tools — Phase A (read-only)
- [ ] Implement MCP Tools — Phase B (export)
- [ ] Implement server admin tools (`onshape_mcp_*`)
- [ ] Build-time collision check for `onshape_mcp_` prefix
- [ ] Write comprehensive unit tests for core crates

### Phase 3: I/O Integration

- [ ] Implement transport layer in `onshape-mcp-io`
- [ ] Implement HTTP client in `onshape-client-io`
- [ ] Wire up in main binary
- [ ] Implement MCP Tools — Phase C (modify)
- [ ] Implement MCP Tools — Phase D (destroy)

### Phase 4: Polish

- [ ] Add documentation
- [ ] Set up coverage reporting
- [ ] Add integration tests
- [ ] Performance testing

### Phase 5: FeatureScript (Future)

- [ ] Implement MCP Tools — Phase E (FeatureScript)
