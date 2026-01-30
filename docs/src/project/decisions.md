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

**Decision:** URL default with optional `save_to` path; `overwrite` parameter; typed errors. See [MCP Tools > Phase B: Export](mcp-tools.md#phase-b-export-mvp).

## CLI Library

**Decision:** `clap` with derive macros; integrates with figment via `Serialized::defaults(Args::parse())`. See [Requirements > Technology Choices](requirements.md#technology-choices).

## Repository Setup

**Decision:** Branch protection, merge queue, signed commits, merge commits only. See [CI > GitHub Repository Settings](ci.md#github-repository-settings).
