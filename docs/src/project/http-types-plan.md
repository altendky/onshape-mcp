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
