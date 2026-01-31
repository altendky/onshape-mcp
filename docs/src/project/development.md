# Development

## Local Development

```bash
# Format
cargo fmt

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Test
cargo test

# Coverage
cargo llvm-cov --all-features --workspace
```

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
| `actionlint` | rhysd/actionlint | pre-commit | GitHub Actions linting |
| `action-validator` | mpalmer/action-validator | pre-commit | Action/workflow schema |
| `cargo fmt --check` | local | pre-commit | Formatting |
| `cargo clippy` | local | pre-commit | Linting |
| `cargo test` | local | manual | Tests |
| `cargo deny` | local | manual | Dependency audit |

**Stages:**

- `pre-commit` — runs automatically on `git commit`
- `manual` — runs only via `pre-commit run --hook-stage manual` or in CI

### Configuration Files

| File | Purpose |
| ------ | --------- |
| `.pre-commit-config.yaml` | Hook definitions |
| `.markdownlint-cli2.yaml` | Markdown linting configuration |
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
