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

See [Data Flow](data-flow.md) for sequence diagrams showing how requests,
authentication, and token refresh flow through each operating mode.

## External Services

```text
┌──────────────────────────────────────────────────────────────────┐
│                     OAuth Token Exchange Proxy                    │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │              workers/oauth-proxy (TypeScript)               │  │
│  │      Cloudflare Worker — forwards token requests to         │  │
│  │      Onshape with client_secret, returns responses          │  │
│  └────────────────────────────────────────────────────────────┘  │
│  onshape-oauth-proxy.fstab.workers.dev                             │
└──────────────────────────────────────────────────────────────────┘
```

The OAuth proxy is a Cloudflare Worker, separate from the Rust workspace.
It holds the OAuth2 client secret and exposes `/token/exchange` and `/token/refresh` endpoints that the CLI calls instead of contacting Onshape's token endpoint directly.
See [OAuth Proxy](oauth-proxy.md) for details.

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
│   ├── onshape-mcp-resources/    # Insight documents (MCP resources)
│   │   ├── Cargo.toml
│   │   ├── build.rs              # Compiles .md insights into Rust constants
│   │   ├── resources -> ../../docs/src/mcp-resources  # SYMLINK
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
│       ├── mcp-resources/
│       │   └── insights/         # Insight .md files (single source of truth)
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
| `onshape-mcp-resources` | Core | Insight documents compiled into MCP resources at build time; `resources/` is a symlink to `docs/src/mcp-resources/` |
| `onshape-mcp-io` | Integration | MCP transport glue (tokio, rmcp); embeds OpenAPI spec via `include_str!()` |
| `onshape-mcp-core` | Core | Pure MCP protocol and business logic, including OpenAPI spec parsing (no I/O) |
| `onshape-client-io` | Integration | HTTP client for Onshape API (reqwest) |
| `onshape-client-core` | Core | Pure Onshape API logic, request/response types (no I/O) |
| `tracing-sansio` | Core | Sans-IO tracing capture library |
| `tracing-sansio-macros` | Core | Proc-macro for `#[capture_tracing]` |
