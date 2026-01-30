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
