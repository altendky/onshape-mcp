# Implementation

## Phase 1: Project Setup

- [x] Initialize Cargo workspace
- [x] Set up crate structure (6 crates: onshape-client-core, onshape-client-io, onshape-mcp-core, onshape-mcp-io, onshape-mcp-resources, onshape-mcp)
- [x] Create rust-toolchain.toml
- [x] Configure linting (workspace Cargo.toml lints, rustfmt.toml)
- [x] Set up pre-commit hooks (.pre-commit-config.yaml, typos.toml)
- [x] Set up GitHub Actions CI (ci.yml with reflow workflows)
- [x] Embed OpenAPI spec (`crates/onshape-mcp-io/onshape-openapi.json`)
- [ ] Create update-openapi-spec.yml workflow
- [x] Add license files (LICENSE-MIT, LICENSE-APACHE)
- [x] Create initial README

## Phase 1.5: Tracing Infrastructure (Deferred)

Sans-IO tracing was planned but has not been started.

- [ ] Create `crates/tracing-sansio/Cargo.toml`
- [ ] Implement `Captured<T>` type and `capture_tracing()` function
- [ ] Create `crates/tracing-sansio-macros/Cargo.toml`
- [ ] Implement `#[capture_tracing]` proc-macro
- [ ] Add unit tests for `capture_tracing()`
- [ ] Add integration tests for `#[capture_tracing]` macro
- [ ] Add documentation with examples

## Phase 2: Core Implementation

- [x] Define effect types in `onshape-mcp-core` (`ToolEffect`, `Continuation`)
- [x] Implement MCP handler state machine (`handle_tool_call`)
- [ ] Implement permission model (modes, escalation)
- [x] Define Onshape API types in `onshape-client-core`
- [x] Implement generic API tools (`onshape_api_search`, `onshape_api_explain`, `onshape_api_call`)
- [x] Implement `onshape_api_schema` tool (schema lookup with polymorphic types, discriminators)
- [x] Implement OpenAPI spec parsing and indexing
- [x] Implement effects-as-data pattern for `ToolResult`
- [x] Implement `onshape_mcp_get_started` tool (onboarding guidance)
- [x] Implement `onshape_auth_status` tool (auth state reporting)
- [x] Implement `onshape_auth_login` tool (OAuth login flow via effects)
- [x] Implement `onshape_screenshot` tool (Part Studio rendering)
- [x] Implement `onshape_error_lookup` tool (FeatureScript error enum resolution)
- [x] Implement resource system (`onshape_list_resources`, `onshape_read_resource`) with compile-time embedded insights
- [x] Implement file reference support in `onshape_api_call` (text, base64, raw bytes encodings)
- [ ] Implement server admin tools (`onshape_mcp_get_mode`, `onshape_mcp_request_mode`) — blocked on permission model
- [ ] Write comprehensive unit tests for core crates

## Phase 3: I/O Integration

- [x] Implement stdio transport in `onshape-mcp-io`
- [x] Implement Streamable HTTP transport (`onshape-mcp http` subcommand) with per-user OAuth
- [x] Implement HTTP client in `onshape-client-io`
- [x] Wire up `onshape_api_call` effect execution in I/O layer
- [x] Wire up in main binary
- [x] Implement `auth login` CLI subcommand (local callback server, PKCE, CSRF)
- [x] Implement token file watching (inotify/kqueue/ReadDirectoryChanges with polling fallback)
- [x] Implement proactive and reactive OAuth token refresh (direct and proxy modes)
- [x] Implement config file permission enforcement (0600 on Unix)

## Phase 4: Polish

- [x] Add documentation (project docs, knowledge base, MCP resource insights)
- [x] Set up coverage reporting (codecov.yml, cargo-llvm-cov workflow)
- [ ] Add integration tests
- [ ] Performance testing

## Phase 5: Distribution

- [x] Create npm wrapper package (`npm/onshape-mcp/`) with cross-platform binary downloads
- [x] Create OpenCode auth plugin (`npm/opencode-auth/`)
- [x] Set up cross-platform release builds (Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64)
- [x] Set up GitHub Release automation (archives, SHA256SUMS)
- [x] Set up crates.io publishing
- [x] Set up npm publishing (with staging validation)
- [x] Create Dockerfile and a historical/private Fly.io deployment for HTTP transport (not publicly offered)

## Phase 6: OAuth Token Exchange Proxy

- [x] Create `workers/oauth-proxy/` project structure (wrangler.toml, package.json, tsconfig.json)
- [x] Implement pure handler (`src/handler.ts`) with request routing and validation
- [x] Implement I/O entry point (`src/index.ts`) as thin effect executor
- [x] Implement `POST /token/exchange` endpoint (with `code_verifier` for PKCE)
- [x] Implement `POST /token/refresh` endpoint
- [x] Implement `GET /health` endpoint
- [x] Implement `GET /config` endpoint (returns client_id)
- [x] Implement `ALLOWED_SOURCES` IP check with DNS-over-HTTPS hostname resolution
- [x] Implement sans-IO test suite (`test/handler.test.ts`)
- [x] Add `proxy_url` support to Rust token data, config, and auth resolution
- [x] Implement `RefreshMethod` enum (Direct/Proxy) in MCP server
- [x] Implement proxy token refresh in `onshape-mcp-io`
- [x] Add proxy-mode auth method to OpenCode plugin
- [x] Historical/private Cloudflare deployment (`onshape-oauth-proxy.fstab.workers.dev`), not publicly offered
- [x] Set secrets via `wrangler secret put` (ONSHAPE_CLIENT_ID, ONSHAPE_CLIENT_SECRET)
- [ ] Configure `ALLOWED_SOURCES` in Cloudflare dashboard
- [ ] Verify all endpoints with curl

## Phase 7: FeatureScript (Future)

- [ ] Implement FeatureScript-related API endpoints (accessible via `onshape_api_call`)
