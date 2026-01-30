# Implementation

## Phase 1: Project Setup

- [ ] Initialize Cargo workspace
- [ ] Set up crate structure
- [ ] Create rust-toolchain.toml
- [ ] Configure linting (clippy.toml, rustfmt.toml)
- [ ] Set up pre-commit hooks (.pre-commit-config.yaml, typos.toml)
- [ ] Set up GitHub Actions CI
- [ ] Set up specs/ directory with OpenAPI spec and license
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
- [ ] Implement MCP Tools — Phase A (read-only)
- [ ] Implement MCP Tools — Phase B (export)
- [ ] Implement server admin tools (`onshape_mcp_*`)
- [ ] Build-time collision check for `onshape_mcp_` prefix
- [ ] Write comprehensive unit tests for core crates

## Phase 3: I/O Integration

- [ ] Implement transport layer in `onshape-mcp-io`
- [ ] Implement HTTP client in `onshape-client-io`
- [ ] Wire up in main binary
- [ ] Implement MCP Tools — Phase C (modify)
- [ ] Implement MCP Tools — Phase D (destroy)

## Phase 4: Polish

- [ ] Add documentation
- [ ] Set up coverage reporting
- [ ] Add integration tests
- [ ] Performance testing

## Phase 5: FeatureScript (Future)

- [ ] Implement MCP Tools — Phase E (FeatureScript)
