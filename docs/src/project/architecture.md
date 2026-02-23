# Architecture

## Layer Design

```text
┌─────────────────────────────────────────────────────────────────┐
│                        Application Layer                         │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                        onshape-mcp                           ││
│  │         (binary - wires everything together)                 ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│                        Integration Layer                         │
│  ┌──────────────────────┐  ┌──────────────────────────────────┐│
│  │   onshape-mcp-io     │  │      onshape-client-io           ││
│  │ (MCP transport glue) │  │   (HTTP client for Onshape)      ││
│  └──────────────────────┘  └──────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│                          Core Layer                              │
│  ┌──────────────────────┐  ┌──────────────────────────────────┐│
│  │   onshape-mcp-core   │  │       onshape-client-core        ││
│  │  (pure protocol &    │  │     (pure Onshape API logic,     ││
│  │   business logic,    │  │      request/response types,     ││
│  │   NO I/O)            │  │      NO I/O)                     ││
│  └──────────────────────┘  └──────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

## Workspace Layout

```text
onshape-mcp/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── onshape-mcp-core/         # Pure MCP logic (sans-IO)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-mcp-io/           # MCP I/O layer (tokio, rmcp)
│   │   ├── Cargo.toml
│   │   ├── onshape-openapi.json  # Vendored Onshape OpenAPI spec
│   │   ├── ONSHAPE-API-LICENSE
│   │   └── src/
│   ├── onshape-client-core/      # Pure Onshape API logic (sans-IO)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-client-io/        # Onshape HTTP client (reqwest)
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── onshape-mcp/              # Main binary
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── tracing-sansio/           # Sans-IO tracing capture
│   │   ├── Cargo.toml
│   │   └── src/
│   └── tracing-sansio-macros/    # Proc-macro for tracing-sansio
│       ├── Cargo.toml
│       └── src/
├── tests/                        # Integration tests
├── .github/
│   └── workflows/
│       ├── ci.yml
│       ├── rust.yml
│       └── update-openapi-spec.yml
├── rust-toolchain.toml
├── .pre-commit-config.yaml
├── typos.toml
├── docs/
│   ├── book.toml
│   └── src/
│       ├── SUMMARY.md
│       └── project/
│           └── *.md
├── README.md
├── LICENSE-MIT
└── LICENSE-APACHE
```

## Crate Descriptions

| Crate | Layer | Purpose |
| ------- | ------- | --------- |
| `onshape-mcp` | Application | Main binary, wires everything together |
| `onshape-mcp-io` | Integration | MCP transport glue (tokio, rmcp); embeds OpenAPI spec via `include_str!()` |
| `onshape-mcp-core` | Core | Pure MCP protocol and business logic, including OpenAPI spec parsing (no I/O) |
| `onshape-client-io` | Integration | HTTP client for Onshape API (reqwest) |
| `onshape-client-core` | Core | Pure Onshape API logic, request/response types (no I/O) |
| `tracing-sansio` | Core | Sans-IO tracing capture library |
| `tracing-sansio-macros` | Core | Proc-macro for `#[capture_tracing]` |
