# HTTP Types Alignment Plan

## Intent

Evaluate and, where appropriate, adopt the Rust `http` crate's standard protocol
types in common Onshape client boundaries.

This should be a distinct future refactor, not bundled with the first binary
response or crate-boundary cleanup work.

## Motivation

The `http` crate provides widely used sans-IO protocol types such as
`HeaderMap`, `HeaderName`, `HeaderValue`, `Method`, `StatusCode`, `Uri`,
`Request`, and `Response`. It is not a network client or server framework, and
it is already a common dependency across `reqwest`, `axum`, `hyper`, and
`tower`-based code.

Using these types may reduce custom HTTP modeling and improve interoperability
without coupling common crates to a specific IO implementation.

## Scope

Research later whether `http` types should replace or supplement local types in:

- response headers.
- request headers.
- status codes.
- methods.
- URLs/URIs, if useful.

Keep this separate from application behavior, token refresh orchestration, MCP
tool logic, and export-service-specific concerns.

## Initial Preference

Prefer adopting `http` types where they improve correctness and interop without
making common APIs awkward to serialize, test, or use as plain sans-IO data.

Do not make this change opportunistically during unrelated refactors. Revisit it
with focused research and implementation when the common response/request model
is otherwise stable.

## Issue 487 Experiment

Issue #487 implemented a focused switch to standard `http` protocol types at the
common client boundary:

- `ApiRequest.method` is `http::Method`.
- `ApiRequest.headers` is `http::HeaderMap`.
- `ApiResponse.status` is `http::StatusCode`.
- `ApiResponse.headers` is `http::HeaderMap`.

The experiment intentionally keeps these local/plain-data choices:

- `ApiRequest.path` remains a relative Onshape API path `String`; applications
  still own the base URL.
- `ApiRequest.query_params` remains `Vec<(String, String)>` for deterministic
  construction and serialization.
- The common client does not wrap requests/responses in `http::Request` or
  `http::Response`.
- MCP tool continuations still receive plain `u16`, header pairs, and bytes; the
  richer common response is converted at the MCP I/O boundary.

Observed tradeoffs:

- `http::Method` removes the local method enum and interops directly with
  `reqwest`, but OpenAPI lowercase method keys require explicit normalization.
- `http::HeaderMap` gives case-insensitive lookup, duplicate-header support, and
  non-UTF-8 value preservation for responses.
- The `http` crate does not expose serde/schema implementations for these types,
  so `ApiRequest` needs custom serde helpers and OpenAPI search/explain DTOs keep
  method values as strings for MCP schemas.
- Request headers are now representable and `onshape-client-io` applies them
  before setting authentication. Caller-supplied `Authorization` is overwritten
  by executor-owned auth, and `Accept: application/json` remains the default when
  no explicit `Accept` is present.
- Dynamic OpenAPI request construction still rejects required header parameters
  because its public input shape has no separate header-parameter map yet; that
  remains part of the request-header/`Accept` follow-up.
