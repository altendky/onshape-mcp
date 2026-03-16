# Principles

## Sans-IO Design

The project follows **full sans-IO design principles** to maximize testability.

### Core Principles

1. **Core crates have zero I/O dependencies** — no tokio, no async, no network, no filesystem
2. **Core crates CAN have pure computation dependencies** — `serde`, `thiserror`, `anyhow`, etc. are allowed
3. **State machines over async/await in core** — pure functions that produce effects
4. **Effects are data** — I/O operations are represented as data structures
5. **I/O crates interpret effects** — thin wrappers that execute the effects
6. **100% testable without mocking** — core logic tested with deterministic inputs

### Example Pattern

As implemented in the generic API tools:

```rust
// Core crate - pure logic, no I/O
pub enum ToolEffect {
    /// Tool completed — no further I/O needed.
    Done(Result<CallToolResult, ErrorData>),
    /// Tool needs an HTTP request to the Onshape API.
    ApiRequest { request: ApiRequest, continuation: Continuation },
    /// Tool requests an OAuth login flow to be started.
    OAuthLoginFlow { mode: LoginMode },
    /// Tool needs files written to disk.
    WriteFiles { files: Vec<FileWrite>, continuation: Continuation },
}

// Plain-data continuation — no closures, Debug-printable
pub enum Continuation {
    FormatApiResponse,
    ProcessAuthValidation { resolved_auth: ResolvedAuth },
    ProcessScreenshotResponse { output_path: PathBuf, label: String, view_matrix: String },
    FormatScreenshotWrite { label: String, view_matrix: String },
}

// Pure function: tool call arguments -> ToolEffect
pub fn call_tool(name: &str, args: Value, /* ... */) -> ToolEffect {
    // Pure logic here, returns either an immediate result
    // or an API request effect for the I/O layer to execute
}

// Pure dispatch: continuation + I/O result -> next effect
pub fn resume(continuation: Continuation, result: IoResult<'_>) -> (ToolEffect, Vec<SideEffect>) {
    // Each Continuation variant matches with the appropriate IoResult variant
    // and produces the next ToolEffect. No closures, fully inspectable.
    // ...
}

// I/O crate - interprets effects in a loop
loop {
    match current {
        ToolEffect::Done(result) => return result,
        ToolEffect::OAuthLoginFlow { mode } => {
            return handle_oauth_login(mode).await;
        }
        ToolEffect::ApiRequest { request, continuation } => {
            let response = http_client.execute(request).await;
            let (next, side_effects) = resume(
                continuation,
                IoResult::ApiResponse { status: response.status, body: &response.body },
            );
            apply_side_effects(side_effects).await;
            current = next;
        }
        ToolEffect::WriteFiles { files, continuation } => {
            let results = write_files(&files).await;
            let (next, side_effects) = resume(continuation, IoResult::FileWriteResults(&results));
            apply_side_effects(side_effects).await;
            current = next;
        }
    }
}
```

## Dependencies Policy

### Core Crates (sans-IO)

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

### I/O Crates

**Allowed:**

- `tokio`
- `rmcp`
- `reqwest`
- Transport-specific crates
