# Error Handling

## Strategy

Use `thiserror` for public API errors (typed, matchable) with `anyhow` for internal convenience and context chaining.

## Onshape API Errors

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

## HTTP Status Code Mapping

| HTTP Code | Error Type |
| ----------- | ------------ |
| 401 | `AuthenticationFailed` |
| 403 | `PermissionDenied` |
| 404 | `NotFound` |
| 408 | `Timeout` |
| 429 | `RateLimited` |
| 500, 503 | `ServerError` |

## Error Context

All errors include context for debugging:

| Field | Description |
| ------- | ------------- |
| `request_id` | Onshape request ID from response headers |
| `endpoint` | API endpoint called |
| `timestamp` | When the error occurred |

## MCP Error Mapping

Hybrid approach: standard JSON-RPC codes where they fit, custom codes for actionable distinctions.

| Error Type | MCP Code | Rationale |
| ------------ | ---------- | ----------- |
| `AuthenticationFailed` | `-32603` | Internal error |
| `PermissionDenied` | `-32603` | Internal error |
| `NotFound` | `-32602` | Invalid params (bad ID) |
| `RateLimited` | `-32000` | Custom: retryable |
| `Timeout` | `-32000` | Custom: retryable |
| `ServerError` | `-32603` | Internal error |
| `CadOperationFailed` | `-32603` | Internal error |

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
