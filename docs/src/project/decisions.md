# Decisions

Resolved design decisions and their rationale.

## Onshape API Functionality

**Decision:** See [MCP Tools](mcp-tools.md) for initial API exposure.

## Configuration Precedence

**Decision:** Defaults → Config file → Env vars → CLI flags

## Authentication Strategy

**Decision:** API Keys initially, OAuth future; config file with permission checks; validation on startup + periodic. See [Authentication](authentication.md).

## Error Handling Strategy

**Decision:** `thiserror` for public API, `anyhow` for internal; hybrid MCP error codes. See [Error Handling](error-handling.md).

## Configuration Tooling

**Decision:** `figment` for layered config (defaults → file → env → CLI). See [Configuration](configuration.md).

## GitHub App Token

**Decision:** Use GitHub App with `actions/create-github-app-token@v1`; detailed setup during repository configuration. See [CI](ci.md#github-app).

## Coverage Enforcement

**Decision:** 100% target with LCOV exclusions; 2% ratchet for project, 100% enforced for patches. See [Development > Coverage Requirements](development.md#coverage-requirements).

## Export Destination

**Decision:** ~~URL default with optional `save_to` path; `overwrite` parameter; typed errors.~~ Superseded by the generic API tools design — export operations are now handled via `onshape_api_call` like any other API endpoint. See [MCP Tools](mcp-tools.md#onshape-api-tools).

## CLI Library

**Decision:** `clap` with derive macros; integrates with figment via `Serialized::defaults(Args::parse())`. See [Requirements > Technology Choices](requirements.md#technology-choices).

## Repository Setup

**Decision:** Branch protection, merge queue, signed commits, merge commits only. See [CI > GitHub Repository Settings](ci.md#github-repository-settings).

## npm Wrapper Package

**Decision:** Multi-package architecture with platform-specific optional dependencies.

| Topic | Decision |
| ----- | -------- |
| Main package | `onshape-mcp` (unscoped for simpler `npx` usage) |
| Platform packages | `@onshape-mcp/{platform}-{arch}` (scoped) |
| Platforms | linux-x64, linux-arm64, darwin-x64, darwin-arm64, win32-x64 |
| Linux linking | Static musl (single binary works on glibc and musl systems) |
| Execution | Synchronous JS shim with inherited stdio |
| Signal handling | Process group semantics (no manual forwarding) |
| Version sync | Lockstep with Cargo version |

See [npm Wrapper](npm-wrapper.md) for details.

## OAuth Proxy Language

**Decision:** TypeScript (not Rust via `workers-rs`).

The OAuth proxy is a ~120-line Cloudflare Worker that constructs form-encoded HTTP requests and forwards responses.
The only code it could share from the existing Rust crates is one URL constant (`https://oauth.onshape.com/oauth/token`).
Additionally, `onshape-client-core` cannot compile to `wasm32-unknown-unknown` (required for Rust-based Workers) because it depends on the `dirs` crate, which requires filesystem APIs unavailable in WebAssembly.
Refactoring to extract a tiny `no_std` constants crate would add workspace complexity for negligible benefit.

TypeScript is the native Cloudflare Workers language with the simplest tooling (`wrangler dev`/`wrangler deploy`).
The duplicated URL constant is acceptable given the compilation barriers and minimal code overlap.

## OAuth Proxy Access Restriction

**Decision:** `ALLOWED_SOURCES` environment variable with in-worker IP check, configured via the Cloudflare dashboard.

Alternatives considered:

- **Cloudflare Access (Zero Trust):** More robust identity-based access control, but requires the CLI to include service token headers on every request, adding complexity to the CLI side and coupling it to Cloudflare's auth infrastructure.
- **Cloudflare WAF Custom Rules:** IP restriction at the edge with no worker code needed, but requires separate dashboard/API management and does not support hostname resolution for dynamic DNS entries.

The chosen approach keeps everything self-contained: the worker resolves hostnames in `ALLOWED_SOURCES` via DNS-over-HTTPS at request time (handling dynamic DNS automatically), and the configuration lives in the Cloudflare dashboard (not committed to the repository).
Changes to the whitelist take effect immediately without redeployment.

## OAuth Proxy Server-Side Callback

**Decision:** Deferred.
See [OAuth Proxy > Deferred: Server-Side Callback Flow](oauth-proxy.md#deferred-server-side-callback-flow) for the design and security analysis.

## OAuth Proxy Sans-IO Architecture

**Decision:** Split the worker into `handler.ts` (pure logic) and `index.ts` (I/O executor).

The worker's core logic (routing, body validation, IP checking, form-body construction) is expressed as pure functions that return `Effect` descriptors (`JsonResponse` or `ForwardToOnshape`).
The I/O layer (~30 lines) parses the request, resolves DNS, calls the pure handler, and executes the returned effect.

This enables testing the entire handler logic without mocking `fetch`, DNS resolution, or any other I/O.
Tests call pure functions and assert on the returned Effect objects — no mock fidelity concerns.

## OAuth Proxy Mode for Auth Resolution

**Decision:** `proxy_url` is an alternative to `client_secret` for OAuth auth resolution.

The `AuthInventory` accepts `has_proxy_url` as sufficient for OAuth capability (even without `client_id` or `client_secret`).
The proxy URL can come from either the `ONSHAPE_PROXY_URL` environment variable or the token file's `proxy_url` field.
This enables zero-configuration proxy mode: the OpenCode plugin writes `proxy_url` into the token file, and the MCP server detects it automatically.

## OAuth Dual Mode (Direct + Proxy)

**Decision:** Support both direct and proxy OAuth modes simultaneously.

The OpenCode auth plugin offers two auth methods: "Onshape OAuth (via proxy)" and "Onshape OAuth (direct)".
The MCP server detects the mode from the token file (presence of `proxy_url` vs `client_secret`) and refreshes accordingly via a `RefreshMethod::Direct` / `RefreshMethod::Proxy` enum.
Config env var (`ONSHAPE_PROXY_URL`) takes precedence over the token file.
