# Implementation

## Phase 1: Project Setup

- [ ] Initialize Cargo workspace
- [ ] Set up crate structure
- [ ] Create rust-toolchain.toml
- [ ] Configure linting (clippy.toml, rustfmt.toml)
- [ ] Set up pre-commit hooks (.pre-commit-config.yaml, typos.toml)
- [ ] Set up GitHub Actions CI
- [x] Set up specs/ directory with OpenAPI spec and license
- [ ] Create update-openapi-spec.yml workflow
- [ ] Configure GitHub App for CI triggering
- [ ] Add license files
- [ ] Create initial README

## Phase 1.5: Tracing Infrastructure

- [ ] Create `crates/tracing-sansio/Cargo.toml`
- [ ] Implement `Captured<T>` type and `capture_tracing()` function
- [ ] Create `crates/tracing-sansio-macros/Cargo.toml`
- [ ] Implement `#[capture_tracing]` proc-macro
- [ ] Add unit tests for `capture_tracing()`
- [ ] Add integration tests for `#[capture_tracing]` macro
- [ ] Add documentation with examples

## Phase 2: Core Implementation

- [ ] Define effect types in `onshape-mcp-core`
- [ ] Implement MCP handler state machine
- [ ] Implement permission model (modes, escalation)
- [ ] Define Onshape API types in `onshape-client-core`
- [x] Implement generic API tools (`onshape_api_search`, `onshape_api_explain`, `onshape_api_call`)
- [x] Implement OpenAPI spec parsing and indexing
- [x] Implement effects-as-data pattern for `ToolResult`
- [ ] Implement server admin tools (`onshape_mcp_get_mode`, `onshape_mcp_request_mode`)
- [ ] Write comprehensive unit tests for core crates

## Phase 3: I/O Integration

- [ ] Implement transport layer in `onshape-mcp-io`
- [ ] Implement HTTP client in `onshape-client-io`
- [ ] Wire up `onshape_api_call` effect execution in I/O layer
- [ ] Wire up in main binary

## Phase 4: Polish

- [ ] Add documentation
- [ ] Set up coverage reporting
- [ ] Add integration tests
- [ ] Performance testing

## Phase 5: FeatureScript (Future)

- [ ] Implement FeatureScript-related API endpoints (accessible via `onshape_api_call`)
