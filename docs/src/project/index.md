# Onshape MCP

A Rust-based MCP (Model Context Protocol) server for Onshape integration. The project emphasizes testability through sans-IO design principles and comprehensive cross-platform support.

## Documentation

### Core Design

- [Principles](principles.md) — Sans-IO philosophy, testability goals, dependencies policy
- [Architecture](architecture.md) — Layer design, crate structure, workspace layout
- [Common Onshape Client Extraction Plan](common-onshape-client-plan.md) — Incremental plan for neutral common client crates
- [HTTP Types Alignment Plan](http-types-plan.md) — Future evaluation of standard `http` crate protocol types
- [Requirements](requirements.md) — Platform support, technology choices, toolchain

### Features

- [MCP Tools](mcp-tools.md) — Tool specifications, permission model, Onshape API phases
- [Configuration](configuration.md) — Config sources, precedence, settings reference
- [Authentication](authentication.md) — Auth methods, credentials, security
- [Error Handling](error-handling.md) — Error types, HTTP mapping, MCP error codes
- [Tracing](tracing.md) — Sans-IO tracing design (tracing-sansio)

### Infrastructure

- [CI](ci.md) — GitHub settings, workflows, coverage, OpenAPI management
- [Development](development.md) — Local development, pre-commit hooks, testing
- [Release](release.md) — Distribution, versioning, publishing
- [npm Wrapper](npm-wrapper.md) — npm package design for `npx` installation
- [OAuth Proxy](oauth-proxy.md) — Cloudflare Worker token exchange proxy for OAuth2

### Knowledge Pipeline

- [Knowledge Pipeline](knowledge-pipeline/index.md) — Self-supervised methodology for building CAD API knowledge
- [Knowledge Base](../knowledge/index.md) — Layered source knowledge produced by the pipeline

### Project Management

- [Implementation](implementation.md) — Phase checklist, roadmap
- [Decisions](decisions.md) — Resolved design decisions
- [Open Questions](open-questions.md) — Pending decisions, deferred items
- [Contributing](contributing.md) — Contribution guidelines
- [Changelog](changelog.md) — Version history
