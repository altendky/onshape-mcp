# Common Onshape Client Extraction Plan

## Goal

Prepare `onshape-mcp` for a future project-neutral Onshape Rust client/library
without a wholesale move or rewrite.

The common boundary should be useful to both `onshape-mcp` and a future
`onshape-export` service. It must stay strictly sans-IO where appropriate: core
logic may model IO, request IO, and consume IO results, but it must not perform
network, storage, clock, filesystem, or framework IO directly.

## Progress Tracking

For isolated implementation sessions, use the first unchecked item below as the
next step. After implementing and verifying that step, mark it complete in this
section. If a step is blocked, leave it unchecked and add a short note explaining
what decision or dependency is missing.

- [x] 1. Add byte/header-capable responses.
- [x] 2. Update the reqwest executor.
- [x] 3. Adapt MCP at the boundary.
- [x] 4. Move MCP defaults and persistence metadata out of common OAuth.
- [ ] 5. Clean common crate documentation and names.
- [ ] 6. Strengthen OpenAPI tests before moving.
- [ ] 7. Isolate OpenAPI internally.
- [ ] 8. Extract `onshape-openapi` only after second use.
- [ ] 9. Add export-oriented common helpers later.
- [ ] 10. Move to a common repo last.

## Constraints

- Prefer small, behavior-preserving refactors inside `onshape-mcp` first.
- Avoid moving crates to a separate repo until there is a second real consumer.
- Keep `onshape-mcp` behavior unchanged while refactoring.
- Preserve dependency direction: common crates must not depend on MCP crates.
- Keep cache identity, job orchestration, storage, Fly, Tigris/S3, and axum logic
  out of common Onshape client crates.

## Decisions So Far

- Use buffered response bodies now: `ApiResponse` should own response bytes in a
  `Vec<u8>`. If large artifact downloads require streaming later, add a separate
  IO-layer API such as `execute_streaming`; do not model streams in core now.
- Keep `OnshapeClient::execute` as the normal buffered executor name.
- Keep `ApiRequest` relative to an application-provided Onshape API base URL.
  Do not allow arbitrary absolute URLs in the normal authenticated API request
  type.
- Do not add request header or `accept` fields in the first response refactor.
  Preserve the existing JSON-oriented request behavior until a concrete need is
  proven.
- Until request headers are intentionally modeled, OpenAPI request building must
  not silently drop header parameters. Required header parameters should make
  request construction fail with a clear unsupported-parameter error.
- Represent response headers as plain `Vec<(String, String)>` for the first
  response refactor, with helper methods for common lookups. Revisit standard
  `http` crate protocol types as a separate planned effort.
- Convert buffered response bytes to text only at the MCP boundary. Existing MCP
  text/JSON continuations should use strict UTF-8 decoding; invalid UTF-8 should
  become an explicit error instead of lossy text.
- Keep `ClientAuthConfig` in `onshape-client-io`, not `onshape-client-core`.
  Core owns credential/token/header helpers; IO owns executor configuration.
- Keep the `oauth2` dependency in `onshape-client-core` for now, isolated to
  `auth` and `oauth`. Do not block extraction on feature-gating or splitting
  OAuth support.
- Split OAuth token material from app-specific persistence metadata. Common code
  should have a smaller token-material type, while MCP owns metadata such as
  `client_id`, `client_secret`, and `proxy_url`. Preserve the existing flat MCP
  token-file JSON shape during this internal split.
- Preserve MCP compatibility with `proxy_url` values already stored in token
  files, but keep that trust decision MCP-specific. Common/reusable OAuth types
  should not treat persisted proxy selection as a generic credential policy.
- Defer the concrete `onshape-export` auth mode. Common APIs should support
  authenticated request execution without owning application-specific credential
  discovery, storage, refresh orchestration, or UX.
- Keep token refresh orchestration app-owned. Common IO executes requests with
  the auth material it is given; pure refresh decision helpers may remain in
  core. Shared pure helpers should cover the common proactive-refresh and
  retry-on-401 decisions so MCP stdio OAuth and per-user HTTP OAuth do not drift.
- Put future typed Onshape endpoint helpers in `onshape-client-core::endpoints::*`
  initially, while keeping them sans-IO and separate from low-level primitives.
  Helpers should cite the OpenAPI operation ID they implement and have golden
  request tests for path/query/body construction.
- Do not make `onshape-export` depend on dynamic OpenAPI request building for
  normal operation. Prefer typed pure endpoint helpers for future export
  workflows; keep OpenAPI support optional.
- Do not define an implicit common Onshape API base URL. Applications must
  provide the base URL explicitly. Extracted OpenAPI code should not silently
  fall back to an old Onshape API version. MCP may keep its current embedded-spec
  fallback as a compatibility detail until OpenAPI code is extracted.
- Keep the embedded OpenAPI spec in `onshape-mcp-io` for now. If an
  `onshape-openapi` crate is extracted, it should parse caller-provided JSON and
  should not force all consumers to embed the full spec.

## Current Boundary Assessment

### Common Now

- `crates/onshape-client-core/src/request.rs`
- `ApiRequest`, `HttpMethod`, `RequestBody`, `MultipartBody`, `BinaryField`
- `ApiResponse`, after it is changed from text-only to bytes/header-capable
- `crates/onshape-client-core/src/auth.rs`
- `Credentials`
- Basic authorization header construction
- Bearer authorization header construction
- `crates/onshape-client-core/src/oauth.rs`
- Onshape OAuth endpoint constants and client builder
- OAuth token material and pure token lifecycle helpers
- `OAuthSession`, `PreExecuteAction`, `PostExecuteAction`
- OAuth login state-machine pieces, if kept UI/server-neutral
- `crates/onshape-client-io/src/lib.rs`
- Thin `reqwest` executor
- Base URL/auth/timeout config
- Request execution and multipart upload support

### Onshape-Specific But Not MCP-Specific

- Onshape API base URL conventions.
- Onshape OAuth endpoints.
- Onshape OpenAPI request building/search/schema lookup, if extracted.
- Translation/export request construction and polling helpers, when added.
- External-data download request construction, when added.
- Document-version Part Studio and Assembly helpers, when added.
- Configuration parameter discovery and encoding helpers, when added.

### MCP-Specific

- `crates/onshape-mcp-core/src/tools.rs`
- Tool definitions, `ToolEffect`, continuations, `CallToolResult`, `rmcp::Content`,
  screenshot file effects, and auth status UX.
- MCP `file_refs` path validation, local file reads, and file-content injection.
  Common upload/request helpers should model binary data, not local file paths.
- `crates/onshape-mcp-core/src/config.rs`
- App config shape, MCP HTTP transport config, auth inventory, and status
  presentation.
- `crates/onshape-mcp-io/*`
- MCP transport/server integration.
- Local token-file watching.
- OAuth callback/login server.
- Per-user MCP OAuth server.
- OpenCode/plugin-facing login and status behavior.
- Embedded OpenAPI spec location, for now.
- `crates/onshape-mcp-resources`.

### Export-Service-Specific

- Catalog schema.
- Cache identity.
- Job orchestration.
- Tigris/S3 storage.
- Fly/axum runtime.
- Background workers.
- Export artifact manifests and failure records.
- Route handlers and public UX.

## OpenAPI Direction

Do not immediately extract `openapi.rs` into a separate repo crate.

Recommended near-term path:

- Keep `openapi.rs` in `onshape-mcp-core` while cleaning its boundary.
- Ensure it depends only on `onshape-client-core` and general-purpose crates, not
  MCP result or transport types.
- Stop re-exporting response types from the OpenAPI module; request building does
  not need response modeling.
- Extract to a workspace crate only when `onshape-export` has a real need for the
  same search/explain/schema/request-building behavior.

If extracted, prefer an Onshape-oriented crate named `onshape-openapi`, not a
generic OpenAPI crate. The code handles general OpenAPI mechanics, but it is
currently shaped around Onshape details such as the Onshape default server,
`BT...` schema conventions, `btType`, and `x-bttype-options` annotations.

## Eventual Target Structure

Common repo, eventual:

```text
crates/onshape-client-core
  request.rs
  response.rs
  auth.rs
  oauth.rs
  endpoints/*                 # typed pure helpers added only when needed

crates/onshape-client-io
  reqwest executor for onshape-client-core requests
  auth header application
  buffered response byte/header capture

crates/onshape-openapi        # optional after second consumer exists
  parse/search/explain/build_request
  Onshape OpenAPI schema helpers
  no required embedded spec; callers provide spec JSON
```

`onshape-mcp`, after refactor:

```text
crates/onshape-mcp-core
  MCP tool definitions
  MCP continuations/effects
  MCP auth status UX
  depends on onshape-client-core
  eventually depends on onshape-openapi

crates/onshape-mcp-io
  MCP stdio/http transports
  token-file watching
  OAuth login/callback server
  embedded OpenAPI spec loading until moved
  executes ToolEffect IO

crates/onshape-mcp-resources
  MCP resource documents
```

Future `onshape-export`:

```text
crates/onshape-export-core
  catalog/cache/job/export state machines
  artifact identity
  pure request construction decisions

crates/onshape-export-server or onshape-export-io
  axum
  Tigris/S3
  Onshape client execution
  background workers
```

## Migration Plan

### 1. Add Byte/Header-Capable Responses

Change the common response model before any extraction.

Current issue: `onshape-client-io` reads responses with `.text().await`, which is
insufficient for GLB/glTF, STEP, STL, 3MF, and other binary artifacts.

Target shape:

```rust
pub struct ApiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ResponseBody,
}

pub struct ResponseBody {
    pub bytes: Vec<u8>,
}
```

Add convenience helpers such as `as_bytes()`, `text()`, `text_lossy()`, and
`content_type()` as needed. Use plain `Vec<(String, String)>` response headers
for this step; defer possible `http::HeaderMap` adoption to the separate HTTP
types alignment effort. Keep HTTP error statuses as successful transport
responses; callers decide how to interpret them.

### 2. Update the Reqwest Executor

- Change response reading from `.text().await` to `.bytes().await`.
- Capture response headers as neutral `(String, String)` pairs, not
  `reqwest::HeaderMap`.
- Preserve existing request behavior, including the current JSON-oriented
  `Accept` header.
- Do not add generic request headers or an `accept` field in this first response
  refactor.
- Dynamic OpenAPI request building should reject endpoints with required header
  parameters for now, because the common request type cannot represent them yet.
  Do not silently omit required headers.
- If future export artifact downloads require different `Accept` behavior, add
  that as a focused follow-up once the concrete endpoint behavior is known.

### 3. Adapt MCP at the Boundary

Keep MCP behavior unchanged by converting bytes to text only at the MCP edge.

The smallest safe step is:

- Let `onshape-client-io` return bytes.
- Let `onshape-mcp-io::RawResponse` store bytes and headers.
- Convert to text with strict UTF-8 when resuming existing MCP continuations that
  currently expect `&str`.
- Treat invalid UTF-8 as an explicit MCP edge error. Do not use lossy decoding for
  existing text/JSON continuations.

Existing tools primarily expect JSON/text. `onshape_screenshot` currently
receives JSON containing base64 image data, so it does not need binary response
handling yet.

### 4. Move MCP Defaults and Persistence Metadata Out of Common OAuth

`onshape-client-core::oauth` currently contains `default_data_dir()` and
`default_token_file_path()` that hard-code `onshape-mcp` paths. Move these to an
MCP crate before extraction.

Split OAuth token data into common token material and MCP-owned persistence
metadata. The common type should contain only OAuth token fields:

- `access_token`
- `refresh_token`
- `expires_at`
- `token_type`
- `scopes`

Keep these other OAuth pieces in common:

- Onshape OAuth endpoint constants
- Onshape OAuth client builder
- `OAuthSession` and refresh decision types

Move persisted MCP/OpenCode refresh metadata to an MCP-owned wrapper, for
example `McpOAuthTokenFile`, containing:

- common token material.
- `client_id`.
- `client_secret`.
- `proxy_url`.

The split is internal API cleanup, not a token-file migration. Existing token
files are persisted user data, so MCP serialization/deserialization should remain
compatible with the current flat JSON shape unless a separate migration is
explicitly planned.

MCP may continue honoring `proxy_url` persisted in existing token files for
compatibility. That behavior should remain in the MCP-owned wrapper/config layer;
common token material should not encode a reusable policy that persisted proxy
URLs are trusted for refresh-token exchange.

Token refresh orchestration remains application-owned. `onshape-client-io`
should execute requests using the auth material it was configured with; it should
not own token storage, login flows, OAuth pending states, or refresh UX.

Pure refresh decision helpers should be reusable by both current OAuth execution
paths:

- stdio/file-based OAuth using `OAuthSession`.
- HTTP per-user OAuth using server-owned user token state.

This keeps refresh orchestration and persistence app-owned while avoiding drift
in shared decisions such as proactive refresh margins and retry-on-401 behavior.

### 5. Clean Common Crate Documentation and Names

- Remove MCP-specific comments from `onshape-client-core` and
  `onshape-client-io`.
- Keep Onshape-specific naming where accurate.
- Avoid broad renames unless they remove concrete coupling.

### 6. Strengthen OpenAPI Tests Before Moving

Ensure coverage exists for:

- path and query parameter substitution.
- required header parameters are rejected while request headers are unsupported.
- JSON body request building.
- multipart binary and text body building.
- schema lookup with `allOf`.
- discriminator annotation.

These tests mostly exist today and should remain the safety rail for later moves.

### 7. Isolate OpenAPI Internally

- Keep `openapi.rs` in `onshape-mcp-core` initially.
- Remove unnecessary response re-exports.
- Keep dependencies free of `rmcp`, MCP resources, and IO crates.
- Do not silently fall back to a hard-coded Onshape API server URL in extracted
  common code. Applications should provide the base URL or spec JSON explicitly.
  MCP may keep its current fallback internally until extraction, but the fallback
  must not move into a reusable OpenAPI crate.
- Consider moving it to a crate-shaped module name if that improves clarity, but
  avoid churn without a second consumer.

### 8. Extract `onshape-openapi` Only After Second Use

When `onshape-export` needs the same OpenAPI behavior:

- Create `crates/onshape-openapi` in the workspace.
- Move `openapi.rs` with minimal edits.
- Use dependency direction `onshape-openapi -> onshape-client-core`.
- Update `onshape-mcp-core -> onshape-openapi`.
- Keep embedded spec loading outside the crate. `onshape-mcp` may continue using
  `include_str!`, but the common OpenAPI helper should parse caller-provided
  JSON.

### 9. Add Export-Oriented Common Helpers Later

Add only pure Onshape request/response helpers that are actually needed by
`onshape-export`.

Place these initially under `onshape-client-core::endpoints::*`, separate from
low-level `request`, `response`, `auth`, and `oauth` modules. Helpers must build
or parse data only; they must not execute HTTP, read clocks, or access storage.

Candidate common additions:

- Create translation/export request data.
- Poll translation status request data.
- Download external data request data.
- Document-version Part Studio and Assembly endpoint helpers.
- Configuration parameter discovery request helpers.
- Configuration parameter encoding helpers.

When adding typed endpoint helpers, anchor each helper to the Onshape OpenAPI
operation ID it implements and add golden tests for the pure request it builds.
Start with hand-owned helpers; do not introduce generated models or a large
generated client until a concrete export workflow needs them.

Do not add cache, job, storage, or route concepts to common crates.

### 10. Move to a Common Repo Last

Move crates to a neutral common repo only after:

- `onshape-mcp` uses the cleaned common APIs.
- `onshape-export` has a vertical-slice consumer.
- The API shape has survived at least one real export workflow.

Use temporary path or git dependencies if needed during the final extraction.
Avoid public API churn during the repo move itself.

## Verification

Run after each small refactor:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Focused checks during response refactors:

```sh
cargo test -p onshape-client-core
cargo test -p onshape-client-io
cargo test -p onshape-mcp-core
cargo test -p onshape-mcp-io
```

Focused checks during OpenAPI isolation or extraction:

```sh
cargo test -p onshape-mcp-core openapi
# After an onshape-openapi crate exists:
cargo test -p onshape-openapi
cargo clippy -p onshape-openapi --all-targets --all-features -- -D warnings
```

Focused test coverage for this extraction step:

- byte response helpers, including strict UTF-8 failure behavior.
- MCP edge conversion from byte responses into existing text/JSON continuations.
- OpenAPI request building rejects required header parameters while request
  headers are unsupported.
- direct and proxy OAuth token-file fixtures preserve the current flat MCP JSON
  shape through the token-material split.
- shared pure refresh decision helpers cover proactive refresh and retry-on-401
  behavior for both stdio OAuth and HTTP per-user OAuth callers.

Manual behavior checks after MCP edge conversion:

- `onshape_auth_status` with cached and nonvalidated credentials.
- `onshape_api_search`.
- `onshape_api_explain`.
- `onshape_api_call` for a JSON endpoint.
- multipart file upload path, if credentials and a test document are available.
- `onshape_screenshot`.

## Risks

- Bytes response support can subtly change invalid UTF-8 handling in MCP tools.
- Captured headers can contain sensitive values; avoid user-facing raw header
  dumps without filtering.
- The current `Accept: application/json` default may be wrong for artifact
  downloads.
- Splitting OAuth token data incorrectly could break existing MCP token files;
  preserve the current flat JSON shape during the internal type split.
- OpenAPI extraction could accidentally pull MCP dependencies into common code.
- A generic OpenAPI crate would create unnecessary maintenance burden.
- Buffering artifacts into `Vec<u8>` is simple but may be memory-heavy for large
  exports.
- Sans-IO purity can regress if helpers start calling clocks, filesystems,
  storage, or HTTP directly.

## Unresolved Questions

- When `onshape-export` is developed, which concrete Onshape endpoints and
  parameter encodings should become typed common helpers?
- If Onshape external-data downloads require absolute URLs, should they use a
  separate explicit download request type, and what host/auth/redirect rules
  should that type enforce?
- After the binary response model is stable, which local request/response HTTP
  types should be replaced or supplemented by standard `http` crate types?
