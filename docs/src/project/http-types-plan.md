# HTTP Types Alignment Decision

## Intent

Record the adopted use of the Rust `http` crate's standard protocol types in
common Onshape client boundaries.

## Motivation

The `http` crate provides widely used sans-IO protocol types such as
`HeaderMap`, `HeaderName`, `HeaderValue`, `Method`, `StatusCode`, `Uri`,
`Request`, and `Response`. It is not a network client or server framework, and
it is already a common dependency across `reqwest`, `axum`, `hyper`, and
`tower`-based code.

Using these types reduces custom HTTP modeling and improves interoperability
without coupling common crates to a specific IO implementation.

## Adopted Scope

Use standard `http` protocol types in:

- response headers.
- request headers.
- status codes.
- methods.

Keep these local/plain-data choices:

- `ApiRequest.path` remains a relative Onshape API path `String`; applications
  still own the base URL.
- `ApiRequest.query_params` remains `Vec<(String, String)>` for deterministic
  construction and serialization.
- The common client does not wrap requests/responses in `http::Request` or
  `http::Response`.
- MCP tool continuations still receive plain `u16`, header pairs, and bytes; the
  richer common response is converted at the MCP I/O boundary.

Keep this separate from application behavior, token refresh orchestration, MCP
tool logic, and export-service-specific concerns.

## Decision

Issue #487 adopted a focused switch to standard `http` protocol types at the
common client boundary:

- `ApiRequest.method` is `http::Method`.
- `ApiRequest.headers` is `http::HeaderMap`.
- `ApiResponse.status` is `http::StatusCode`.
- `ApiResponse.headers` is `http::HeaderMap`.

## Tradeoffs

- `http::Method` removes the local method enum and interops directly with
  `reqwest`, but OpenAPI lowercase method keys require explicit normalization.
- `http::HeaderMap` gives case-insensitive lookup, duplicate-header support, and
  non-UTF-8 value preservation for responses.
- The `http` crate does not expose serde/schema implementations for these types,
  so `ApiRequest` needs custom serde helpers and OpenAPI search/explain DTOs keep
  method values as strings for MCP schemas.
- Request headers are representable and `onshape-client-io` applies them before
  setting authentication. Caller-supplied `Authorization` is overwritten by
  executor-owned auth, and `Accept: application/json` remains the default when no
  explicit `Accept` is present.
- Dynamic OpenAPI request construction still rejects required header parameters
  because its public input shape has no separate header-parameter map yet; that
  remains part of the request-header/`Accept` follow-up.

Do not revisit this decision without a concrete problem from real common-client
or export-service usage.
