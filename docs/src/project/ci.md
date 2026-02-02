# CI

## GitHub Repository Settings

This section documents the manual configuration required in GitHub repository settings.

### Branch Protection (main)

| Setting | Value |
| --------- | ------- |
| Require PR before merge | Yes |
| Required approvals | 0 (increase when contributors join) |
| Require status checks | Yes — `all` job only |
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
| `.github/workflows/ci.yml` | Entry point, calls reusable workflows, `all` job aggregation |
| `.github/workflows/reflow-library.yml` | Reusable workflow that outputs matrix configuration |
| `.github/workflows/reflow-pre-commit.yml` | Reusable workflow for pre-commit checks (5 platform jobs) |
| `.github/workflows/reflow-rust.yml` | Reusable workflow for Rust checks (15 matrix jobs) |
| `.github/workflows/reflow-coverage.yml` | Reusable workflow for coverage generation (5 platform jobs) |
| `.github/workflows/update-openapi-spec.yml` | Nightly/manual OpenAPI spec update, creates PR (planned) |

## CI Architecture

The CI uses reusable workflows for visual grouping in the GitHub Actions UI. Each reusable workflow appears as a collapsible group containing its matrix jobs.

```text
┌─────────────────────────────────────────────────────────────┐
│                    ci.yml (entry point)                      │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    pre-commit:                                               │
│      uses: ./.github/workflows/reflow-pre-commit.yml        │
│                                                              │
│    rust:                                                     │
│      uses: ./.github/workflows/reflow-rust.yml              │
│                                                              │
│    coverage:                                                 │
│      uses: ./.github/workflows/reflow-coverage.yml          │
│                                                              │
│    all:                                                      │
│      needs: [pre-commit, rust, coverage]                    │
│      uses: re-actors/alls-green                             │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│        reflow-pre-commit.yml (reusable workflow)             │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    library: uses reflow-library.yml                         │
│    check: 5 platform jobs (pre-commit/action)               │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│           reflow-rust.yml (reusable workflow)                │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    library: uses reflow-library.yml                         │
│    check: 15 matrix jobs (3 rust × 5 platforms)             │
│      - cargo fmt, clippy, test, deny                        │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│         reflow-coverage.yml (reusable workflow)              │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    library: uses reflow-library.yml                         │
│    check: 5 platform jobs (cargo-llvm-cov, stable only)     │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│           reflow-library.yml (reusable workflow)             │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    generate:                                                 │
│      uses: ./.github/actions/library                        │
│      outputs: matrix configuration as JSON                   │
└─────────────────────────────────────────────────────────────┘
```

**Note:** Each reusable workflow calls `reflow-library.yml` internally to get the matrix configuration. This results in the library job running 3 times per CI run, but the overhead is minimal.

## Library Action

The CI matrix is generated by a composite action at `.github/actions/library/`.

### Configuration

Matrix configuration is defined in `.github/actions/library/library.yml`:

```yaml
axes:
  os:
    - name: Linux
      matrix: linux
      emoji: 🐧
      runs-on:
        arm: ubuntu-24.04-arm
        intel: ubuntu-latest
    - name: macOS
      matrix: macos
      emoji: 🍎
      runs-on:
        arm: macos-latest
        intel: macos-15-intel
    - name: Windows
      matrix: windows
      emoji: 🪟
      runs-on:
        arm: windows-11-arm
        intel: windows-latest
  arch:
    - name: ARM
      matrix: arm
      emoji: 💪
    - name: Intel
      matrix: intel
      emoji: 🌀

exclude:
  windows-arm:
    - os:
        matrix: windows
      arch:
        matrix: arm
```

The action parses this YAML and outputs it as JSON for use in workflow matrices.

## Platform Matrix

| OS | Architecture |
| ---- | -------------- |
| Linux (ubuntu) | ARM, Intel |
| macOS | ARM, Intel |
| Windows | Intel |

**Note:** Windows ARM is excluded due to insufficient ecosystem support.

### Rust Version Matrix

| Toolchain | Required | Notes |
| ----------- | ---------- | ------- |
| MSRV (1.88) | Yes | From `rust-toolchain.toml` |
| Latest stable | Yes | Primary development target |
| Beta | No | Allowed to fail |

### Job Count

| Workflow | Jobs |
| -------- | ---- |
| Pre-commit | 5 (1 per platform) |
| Rust | 15 (3 rust versions × 5 platforms) |
| Coverage | 5 (stable only × 5 platforms) |
| Library | 3 (called by each reusable workflow) |
| All | 1 |
| **Total** | **29 jobs** |

## CI Tooling

| Tool | Purpose |
| ------ | --------- |
| [pre-commit/action](https://github.com/pre-commit/action) | Runs pre-commit hooks in CI |
| [re-actors/alls-green](https://github.com/re-actors/alls-green) | Aggregate job status |
| [actions-rust-lang/setup-rust-toolchain](https://github.com/actions-rust-lang/setup-rust-toolchain) | Rust toolchain installation |
| [taiki-e/install-action](https://github.com/taiki-e/install-action) | Install cargo tools (cargo-deny, cargo-llvm-cov) |
| [codecov/codecov-action](https://github.com/codecov/codecov-action) | Upload coverage to Codecov |

**GitHub branch protection:** Only the `all` job is required.

## Checks

### Pre-commit Checks

Pre-commit hooks run on all platform combinations.
See [Development > Pre-commit Hooks](development.md#pre-commit-hooks) for the full list of hooks.

| Check | Tool | Run On |
| ------- | ------ | -------- |
| Pre-commit hooks | `pre-commit/action` | All 5 platform combinations |

### Rust Checks

| Check | Tool | Run On |
| ------- | ------ | -------- |
| Formatting | `cargo fmt --check` | All 15 matrix combinations |
| Linting | `cargo clippy` | All 15 matrix combinations |
| Tests | `cargo test` | All 15 matrix combinations |
| Dependency audit | `cargo deny` | All 15 matrix combinations |

### Coverage

| Check | Tool | Run On |
| ------- | ------ | -------- |
| Coverage | `cargo-llvm-cov` | Stable only, all 5 platforms |
| Upload | `codecov/codecov-action` | With OIDC authentication |

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

## Dependency Monitoring

Dependabot is configured to monitor dependencies and create PRs for updates.

| Setting | Value |
| ------- | ----- |
| Location | `.github/dependabot.yml` |
| Schedule | Daily |
| Grouping | None (individual PRs) |

### Monitored Ecosystems

| Ecosystem | Directory | Description |
| --------- | --------- | ----------- |
| `github-actions` | `/` | Actions used in workflows |
| `cargo` | `/` | Rust dependencies |
| `npm` | `/npm/onshape-mcp` | npm wrapper package |

### Not Covered by Dependabot

The following dependency mechanisms require alternative approaches:

| Mechanism | Location | Update Method |
| --------- | -------- | ------------- |
| Pre-commit hooks | `.pre-commit-config.yaml` | `pre-commit autoupdate` or Renovate |
