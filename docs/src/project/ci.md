# CI

## GitHub Repository Settings

This section documents the manual configuration required in GitHub repository settings.

### Branch Protection (main)

| Setting | Value |
| --------- | ------- |
| Require PR before merge | Yes |
| Required approvals | 0 (increase when contributors join) |
| Require status checks | Yes — `alls-green` job only |
| Require merge queue | Yes |
| Require branches up-to-date | No (merge queue handles this) |
| Show update branch button | Always |
| Require signed commits | Yes |
| Include administrators | Yes |

### Merge Queue

Merge queue enabled to guarantee main stays green. PRs merge only after passing CI on the queued merge commit. This replaces the need for "require branches to be up-to-date" which can cause CI thrashing.

### Merge Strategy

| Option | Enabled |
| -------- | --------- |
| Merge commits | Yes (only) |
| Squash merge | No |
| Rebase merge | No |

### Other Settings

| Setting | Value |
| --------- | ------- |
| Description | "A Rust-based MCP (Model Context Protocol) server for Onshape integration." |
| Default branch | `main` |
| Auto-delete head branches | Yes |
| Discussions | Disabled |
| Wiki | Disabled |
| Projects | Disabled |
| Issues | Enabled |
| Pull Requests | Enabled |

### GitHub App

Create a GitHub App for CI to run on auto-generated PRs (e.g., OpenAPI spec updates):

| Setting | Value |
| --------- | ------- |
| Permissions | `contents: write`, `pull-requests: write` |
| Installation | Repository only |
| Webhook | Disabled (not needed) |

Store credentials in repository secrets:

- `APP_ID` — from app settings page
- `APP_PRIVATE_KEY` — contents of generated `.pem` file

## Workflow Structure

| File | Purpose |
| ------ | --------- |
| `.github/workflows/ci.yml` | Entry point, Rust version matrix, alls-green aggregation |
| `.github/workflows/rust.yml` | Reusable workflow, platform matrix, all checks |
| `.github/workflows/update-openapi-spec.yml` | Nightly/manual OpenAPI spec update, creates PR |

## CI Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│                    ci.yml (entry point)                      │
├─────────────────────────────────────────────────────────────┤
│  matrix:                                                     │
│    rust: [1.75, stable, beta]                               │
│                                                              │
│  jobs:                                                       │
│    build:                                                    │
│      uses: ./.github/workflows/rust.yml                     │
│      with:                                                   │
│        rust-version: ${{ matrix.rust }}                     │
│                                                              │
│    alls-green:                                               │
│      needs: [build]                                          │
│      allowed-failures: [beta]                                │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│              rust.yml (reusable workflow)                    │
├─────────────────────────────────────────────────────────────┤
│  inputs:                                                     │
│    rust-version: (required)                                  │
│                                                              │
│  matrix (internal):                                          │
│    os: [ubuntu, macos, windows]                             │
│    arch: [x86_64, aarch64]                                  │
│                                                              │
│  jobs (all 6 platform combinations):                         │
│    - setup-rust-toolchain (with rust-version)               │
│    - cargo fmt --check                                       │
│    - cargo clippy                                            │
│    - cargo test                                              │
│    - cargo deny                                              │
│                                                              │
│  jobs (stable × all 6 platforms):                            │
│    - coverage (cargo-llvm-cov)                              │
└─────────────────────────────────────────────────────────────┘
```

## Version & Platform Matrices

### Rust Version Matrix

| Toolchain | Required | Notes |
| ----------- | ---------- | ------- |
| MSRV (1.75) | Yes | From `rust-toolchain.toml` |
| Latest stable | Yes | Primary development target |
| Beta | No | Allowed to fail |

### Platform Matrix

| OS | Architecture |
| ---- | -------------- |
| Linux (ubuntu) | x86_64, aarch64 |
| macOS | x86_64, aarch64 |
| Windows | x86_64 |

**Note:** Windows aarch64 is excluded due to insufficient ecosystem support.

**Total jobs:**

- Checks: 3 rust × 5 platforms = 15 jobs
- Coverage: 1 rust (stable) × 5 platforms = 5 jobs
- **Total: 20 jobs** (plus alls-green)

## CI Tooling

| Tool | Purpose |
| ------ | --------- |
| [actions-rust-lang/setup-rust-toolchain](https://github.com/actions-rust-lang/setup-rust-toolchain) | Rust installation, reads `rust-toolchain.toml`, caching |
| [re-actors/alls-green](https://github.com/re-actors/alls-green) | Aggregate job status, allow beta failures |

**GitHub branch protection:** Only the `alls-green` job is required.

## Checks

| Check | Tool | Run On |
| ------- | ------ | -------- |
| Formatting | `cargo fmt --check` | All matrix combinations |
| Linting | `cargo clippy` | All matrix combinations |
| Tests | `cargo test` | All matrix combinations |
| Dependency audit | `cargo deny` | All matrix combinations |
| Coverage | `cargo-llvm-cov` | Stable only, all platforms |

## PR Title Enforcement

PR titles will be validated in CI. See [Open Questions > Deferred](open-questions.md#deferred) for details.

## Linting & Formatting

### Linting Configuration

```toml
# clippy.toml or in CI
[lints.clippy]
pedantic = "warn"
nursery = "warn"
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
# Allow in tests
```

**Clippy Lints to Enable:**

- `clippy::pedantic` - additional strictness
- `clippy::nursery` - experimental but useful
- `clippy::unwrap_used` - deny in library code
- `clippy::expect_used` - deny in library code
- `clippy::panic` - deny in library code

### Formatting

- Use `rustfmt` with default settings (or minimal customization)
- Enforce via CI

## Testing & Coverage

See [Development > Testing Strategy](development.md#testing-strategy) and [Development > Coverage Requirements](development.md#coverage-requirements).

## OpenAPI Spec Management

The Onshape OpenAPI specification is stored locally for reference and code generation.

| Setting | Value |
| --------- | ------- |
| Location | `specs/onshape-openapi.json` |
| Source | `https://cad.onshape.com/api/v6/openapi` |
| License | Apache 2.0 (see `specs/ONSHAPE-API-LICENSE`) |
| Format | Pretty-printed JSON |

### Update Workflow

| Trigger | Schedule |
| --------- | ---------- |
| Nightly | 09:00 UTC |
| Manual | workflow_dispatch |

**Tooling:**

- [peter-evans/create-pull-request](https://github.com/peter-evans/create-pull-request) — Creates/updates PR when spec changes
- GitHub App token — Enables CI to run on auto-generated PRs

**Behavior:**

- Downloads latest spec from Onshape API
- Pretty-prints JSON for readable diffs
- Creates PR if changes detected (no PR on empty diff)
- Auto-merge enabled (optional, requires branch protection)
- Branch `automated/update-openapi-spec` deleted after merge
