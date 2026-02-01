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

### Verification

Static linking is verified in CI using `ldd`:

```bash
# Should report "not a dynamic executable" or "statically linked"
ldd target/debug/<binary-name>
# Or for release builds:
# ldd target/release/<binary-name>
```

### Local musl Builds

To build musl-linked binaries locally:

```bash
# Option 1: Use Docker (recommended)
docker run --rm -v "$PWD":/app -w /app rust:alpine \
  cargo build --release

# Option 2: Install musl target (Linux only)
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

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
| `cargo nextest run` | local | manual | Tests |
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
