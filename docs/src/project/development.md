# Development

## Local Development

```bash
# Format
cargo fmt

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Test (using nextest for faster, more reliable test runs)
cargo nextest run --all-features

# Coverage
cargo llvm-cov --all-features --workspace
```

### Installing Development Tools

```bash
# Install cargo-nextest (next-generation test runner)
cargo install cargo-nextest --locked

# Install cargo-llvm-cov (coverage)
cargo install cargo-llvm-cov --locked

# Install cargo-deny (dependency audit)
cargo install cargo-deny --locked
```

## Linux Static Linking

Linux binaries are built with musl libc to produce fully static executables.
This ensures compatibility across all Linux distributions, including:

- glibc-based distributions (Ubuntu, Debian, Fedora, RHEL, etc.)
- musl-based distributions (Alpine, Void Linux, etc.)

### Why Static Linking?

Dynamic linking against glibc creates binaries that may fail on systems with older glibc versions.
Static musl linking eliminates this dependency, producing portable binaries that work everywhere.

### Configuration

Static linking for musl targets is configured in `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]

[target.aarch64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]
```

This uses explicit target triples (rather than `cfg(target_env = "musl")`) to ensure
reliable matching even when `rust-toolchain.toml` triggers toolchain switches during
cargo execution. Applies to both x86_64 and ARM64 architectures.

### Verification

Static linking is verified in CI using the `file` command:

```bash
# Should report "statically linked" or "static-pie linked"
file target/debug/<binary-name>
# Or for release builds:
# file target/release/<binary-name>
```

**Note:** We use `file` instead of `ldd` because musl's `ldd` outputs the loader path
even for static binaries, unlike glibc's `ldd` which says "not a dynamic executable".

### Local musl Builds

To build musl-linked binaries locally:

```bash
# Option 1: Use rust:alpine image (simplest)
docker run --rm -v "$PWD":/app -w /app rust:alpine \
  cargo build --release

# Option 2: Use alpine:latest with rustup (matches CI)
# Allows testing with specific Rust versions (stable/beta/MSRV)
docker run --rm -v "$PWD":/app -w /app alpine:latest sh -c "
  apk add --no-cache curl bash gcc musl-dev
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain beta
  . \$HOME/.cargo/env
  cargo build --release
"

# Option 3: Install musl target natively (Linux only)
rustup target add x86_64-unknown-linux-musl
rustup target add aarch64-unknown-linux-musl  # For ARM64
cargo build --release --target x86_64-unknown-linux-musl
# Or for ARM64:
# cargo build --release --target aarch64-unknown-linux-musl
```

**When to use each approach:**

- **Option 1 (rust:alpine)**: Quick local builds with the latest stable Rust. Simplest approach.
- **Option 2 (alpine:latest + rustup)**: Matches CI configuration. Use this to test with specific Rust versions including beta releases for early compatibility testing.
- **Option 3 (native musl target)**: Fastest builds if you're on Linux and have the musl target installed.

## CI Architecture

The CI workflow separates build and test stages:

1. **Build stage**: Compiles tests and creates archives
   - Linux: Builds in Alpine containers (musl-linked)
   - macOS/Windows: Builds natively

2. **Test stage**: Runs pre-built tests in multiple environments
   - Linux tests run on both glibc (Ubuntu) and musl (Alpine)
   - macOS/Windows tests run natively

This architecture verifies that musl binaries work correctly on glibc systems and vice versa.

### Test Runner

We use [cargo-nextest](https://nexte.st/) for test execution:

- Faster parallel test execution
- Better CI integration with archiving support
- Cleaner output and failure reporting

### Shell Scripts

CI scripts use POSIX shell (`sh`) for Alpine container compatibility:

| Convention | Value |
| ---------- | ----- |
| Shebang | `#!/usr/bin/env sh` |
| Error handling | `set -eux` (POSIX: exit on error, undefined vars, trace) |
| Location | `.github/scripts/` and `.github/actions/*/setup.sh` |

**Why POSIX shell?** Alpine Linux uses BusyBox ash, not bash. Scripts that run in
Alpine containers must be POSIX-compatible. The `#!/usr/bin/env sh` shebang ensures
portability across environments.

**Exception:** `resolve.sh` uses `#!/usr/bin/env bash` because it runs on Ubuntu
runners (not in containers) and benefits from bash features like `pipefail`.

## Pre-commit Hooks

**Philosophy:** Pre-commit hooks provide developers an opt-in mechanism for fast local feedback. They do **not** enforce policy — CI is the source of truth.

**Tool:** [pre-commit](https://pre-commit.com/)

### Hook Configuration

| Hook | Source | Stage | Purpose |
| ------ | -------- | ------- | --------- |
| `trailing-whitespace` | pre-commit-hooks | pre-commit | Clean whitespace |
| `end-of-file-fixer` | pre-commit-hooks | pre-commit | Consistent EOF |
| `check-toml` | pre-commit-hooks | pre-commit | TOML syntax |
| `check-yaml` | pre-commit-hooks | pre-commit | YAML syntax |
| `check-merge-conflict` | pre-commit-hooks | pre-commit | Catch conflict markers |
| `typos` | crate-ci/typos | pre-commit | Spell checking |
| `markdownlint-cli2` | DavidAnson/markdownlint-cli2 | pre-commit | Markdown linting |
| `lychee` | lycheeverse/lychee | pre-commit | Link validation |
| `actionlint` | rhysd/actionlint | pre-commit | GitHub Actions linting |
| `action-validator` | mpalmer/action-validator | pre-commit | Action/workflow schema |
| `cargo fmt --check` | local | pre-commit | Formatting |
| `cargo clippy` | local | pre-commit | Linting |
| `cargo nextest run --all-features` | local | manual | Tests |
| `cargo deny` | local | manual | Dependency audit |

**Stages:**

- `pre-commit` — runs automatically on `git commit`
- `manual` — runs only via `pre-commit run --hook-stage manual` or in CI

### Configuration Files

| File | Purpose |
| ------ | --------- |
| `.pre-commit-config.yaml` | Hook definitions |
| `.markdownlint-cli2.yaml` | Markdown linting configuration |
| `.lychee.toml` | Link validation configuration |
| `typos.toml` | Spell check word allowlist |

## Testing Strategy

| Test Type | Location | Coverage Target |
| ----------- | ---------- | ----------------- |
| Unit tests | `crates/*/src/**/*.rs` | 100% with exclusions |
| Integration tests | `tests/` | Key workflows |
| Doc tests | Inline | All public APIs |

## Coverage Requirements

- **Tool:** `cargo-llvm-cov`
- **Reporting:** Codecov

### Philosophy

**Target 100% coverage** with explicit exclusions for untestable code. The sans-IO architecture makes this achievable for core crates. Per-crate targets may be adjusted if specific crates prove less testable.

### Enforcement Strategy

| Check | Behavior |
| ------- | ---------- |
| Project coverage | Ratchet — fail if drops more than 2% from main (catches accidental loss of code/tests) |
| Patch coverage | 100% enforced — new code must be fully covered or explicitly excluded |

Codecov configuration:

```yaml
coverage:
  status:
    project:
      default:
        threshold: 2%  # Catch accidental loss of code/tests
    patch:
      default:
        target: 100%  # Enforced; use LCOV exclusions for untestable code
```

### Coverage Exclusions

Use LCOV comments to exclude code from coverage **with justification**:

```rust
// Platform-specific debug output not testable in CI
println!("debug: {}", value); // LCOV_EXCL_LINE

// LCOV_EXCL_START — Unreachable error path; requires external service failure
Err(e) => {
    log::error!("Unexpected: {}", e);
    panic!("invariant violated");
}
// LCOV_EXCL_STOP
```

**Policy:** Every exclusion must have a comment explaining why the code cannot or should not be tested. Enforced via code review.

## Documentation

### Tooling

| Tool | Purpose |
| ------ | --------- |
| rustdoc | API documentation from source |
| mdBook | Prose documentation (design docs, guides) |

### Structure

```text
docs/
├── book.toml
└── src/
    ├── SUMMARY.md
    ├── project/
    │   ├── principles.md      # Core philosophy (sans-IO, etc.)
    │   ├── architecture.md    # High-level design
    │   └── implementation.md  # Implementation plan/roadmap
    └── (future: user-guide/, etc.)
```

### Standards

*Still to discuss — see [Open Questions](open-questions.md#pending):*

- Rustdoc coverage expectations
- README content and structure
- Usage examples
