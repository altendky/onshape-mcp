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
pub enum ToolResult {
    /// Tool completed immediately with no I/O needed.
    Immediate(Result<CallToolResult, ErrorData>),
    /// Tool needs an HTTP request to the Onshape API.
    OnshapeApiRequest { request: ApiRequest },
}

// Pure function: tool call arguments -> ToolResult
pub fn call_tool(name: &str, args: Value, ...) -> ToolResult {
    // Pure logic here, returns either an immediate result
    // or an API request effect for the I/O layer to execute
}

// I/O crate - interprets effects
match tool_result {
    ToolResult::Immediate(result) => send_response(result),
    ToolResult::OnshapeApiRequest { request } => {
        let response = http_client.execute(request).await;
        let result = process_api_response(response);
        send_response(result);
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
