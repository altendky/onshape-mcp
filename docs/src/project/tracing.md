# Tracing

## Rationale

The standard `tracing` library relies on global/thread-local subscriber state, which conflicts with sans-IO principles. After researching the ecosystem:

| Crate | Purpose | Sans-IO Compatible |
| ------- | --------- | ------------------- |
| `tracing` | Core instrumentation macros | No (global state) |
| `tracing-tunnel` | Serializable event capture | Yes |
| `tracing-capture` | Testing/inspection | Partial |

`tracing-tunnel` provides the foundation:

- `TracingEventSender` - subscriber that serializes events to a callback
- `TracingEvent` - full span hierarchy (enter/exit/close, parent-child relationships)
- `TracingEventReceiver` - replays events to real subscribers

A helper crate `tracing-sansio` wraps this with minimal boilerplate.

## API Design

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

## Crate Structure

```text
crates/
├── tracing-sansio/           # Core library
│   ├── Cargo.toml
│   └── src/lib.rs
└── tracing-sansio-macros/    # Proc-macro (optional feature)
    ├── Cargo.toml
    └── src/lib.rs
```

| Crate | Purpose |
| ------- | --------- |
| `tracing-sansio` | `Captured<T>` type, `capture_tracing()` function, re-exports `TracingEvent` |
| `tracing-sansio-macros` | `#[capture_tracing]` attribute macro |

## Key Types

| Type | Description |
| ------ | ------------- |
| `Captured<T>` | Wrapper containing `result: T` and `events: Vec<TracingEvent>` |
| `TracingEvent` | Re-exported from `tracing-tunnel`; represents spans and events |

## Dependencies

**`tracing-sansio`:**

- `tracing` ^0.1
- `tracing-tunnel` ^0.1 (with `sender` feature)
- `tracing-sansio-macros` (optional, default enabled)

**`tracing-sansio-macros`:**

- `syn` ^2, `quote` ^1, `proc-macro2` ^1

## Limitations

| Limitation | Workaround |
| ------------ | ------------ |
| Sync functions only (macro) | Use `capture_tracing()` closure for async |
| Thread-local capture | Events from spawned threads not captured |
| Bounded channel (default 256) | Use `capture_tracing_with_capacity()` for high-volume |

## Integration Pattern

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

## Future Enhancements

See [Open Questions > Deferred](open-questions.md#deferred) for planned improvements:

- Independent publication to crates.io
- Async support
- Event filtering
