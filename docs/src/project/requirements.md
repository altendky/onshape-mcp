# Requirements

## Platform Support

| Platform | Architecture | Status |
| ---------- | -------------- | -------- |
| Linux | x86_64 | Required |
| Linux | aarch64 | Required |
| macOS | x86_64 | Required |
| macOS | aarch64 | Required |
| Windows | x86_64 | Required |
| Windows | aarch64 | Deferred (ecosystem support insufficient) |

**Constraints:**

- No platform-specific code without abstraction
- No explicit constraints against supporting additional platforms
- All code must compile for all target platforms

## Technology Choices

| Component | Choice | Rationale |
| ----------- | -------- | ----------- |
| MCP SDK | `rmcp` (official Rust SDK) | Official implementation, maintained, tokio-based |
| Async Runtime | Tokio | Required by rmcp, best ecosystem support |
| Configuration | `figment` | Layered config with excellent error provenance, first-class serde/clap integration |
| CLI | `clap` | Derive macros, env var support, shell completions, integrates with figment |
| Minimum Rust Version | 1.75+ | Stable async traits, impl Trait in traits |
| License | MIT OR Apache-2.0 | Standard dual license for Rust projects |

## Toolchain

| File | Purpose |
| ------ | --------- |
| `rust-toolchain.toml` | Pin toolchain version and components |

**Configuration:**

- **Channel:** MSRV (`1.75`)
- **Components:** `rustfmt`, `clippy`, `llvm-tools-preview`

Pinning to MSRV ensures developers default to the minimum supported version.
