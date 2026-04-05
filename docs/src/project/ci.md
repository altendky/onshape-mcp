# CI

## GitHub Repository Settings

This section documents the manual configuration required in GitHub repository settings.

### Branch Protection (main)

| Setting | Value |
| --------- | ------- |
| Require PR before merge | Yes |
| Required approvals | 0 (increase when contributors join) |
| Require status checks | Yes — `all` job only |
| Require merge queue | No (Mergify handles this) |
| Require branches up-to-date | No (Mergify merge queue handles this) |
| Show update branch button | Always |
| Require signed commits | Yes |
| Include administrators | Yes |

### Merge Queue (Mergify)

[Mergify](https://mergify.com) provides merge queue functionality since GitHub's native merge
queue is not available on personal repositories. Mergify is configured via `.mergify.yml` in
the repository root.

**How it works:**

1. Apply the `queue` label to a PR
2. Once CI passes on the PR branch (`check-success = all`), Mergify auto-queues it
3. Mergify creates a temporary branch merging the PR into `main` and runs CI again
4. If CI passes on the merged branch, Mergify merges the PR
5. If CI fails, the PR is dequeued

This two-step CI pattern catches real errors on the PR itself (before entering the queue)
while only infrastructure flakes need to be retried in the queue.

**Flaky failure handling:**

Mergify CI Insights Auto-Retry is configured (via the Mergify dashboard) to automatically
retry failed CI jobs up to 2 times. This handles transient infrastructure failures (runner
provisioning, network timeouts, rate limits) without manual intervention.

| Setting | Value |
| ------- | ----- |
| Configuration | `.mergify.yml` |
| Queue entry | `queue` label + CI green |
| Merge gate | `check-success = all` |
| Merge method | Merge commits |
| Checks timeout | 90 minutes |
| Parallel checks | 1 (no speculative checks) |
| Auto-retry | 2 retries (dashboard-configured) |

### CI Insights

Mergify CI Insights provides test-level analytics by ingesting JUnit XML reports from CI runs.
This enables flaky test detection, auto-retry for infrastructure failures, and test performance
tracking.

**Test report upload:**

The Rust test workflow (`reflow-rust.yml`) uploads JUnit XML reports to Mergify CI Insights
after each test job. Nextest generates JUnit output via the `ci` profile configured in
`.config/nextest.toml`. The `mergifyio/gha-mergify-ci` action uploads the report using the
`MERGIFY_TOKEN` secret.

| Setting | Value |
| ------- | ----- |
| Nextest config | `.config/nextest.toml` |
| Nextest profile | `ci` |
| JUnit output path | `target/nextest/ci/junit.xml` |
| Upload action | `mergifyio/gha-mergify-ci@6875ab3991ec1db831576df1cd00a7870603aa9e # v8` |
| Secret | `MERGIFY_TOKEN` (application key with `ci` scope) |

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
| `.mergify.yml` | Mergify merge queue and autoqueue configuration |
| `.github/workflows/ci.yml` | Entry point for CI and releases, calls reusable workflows, `all` job aggregation |
| `.github/workflows/reflow-library.yml` | Reusable workflow that outputs matrix configuration |
| `.github/workflows/reflow-pre-commit.yml` | Reusable workflow for pre-commit checks (5 platform jobs) |
| `.github/workflows/reflow-rust.yml` | Reusable workflow for Rust lint, build, and test jobs |
| `.github/workflows/reflow-coverage.yml` | Reusable workflow for Rust coverage generation (5 platform jobs) |
| `.github/workflows/reflow-npm.yml` | Reusable workflow for npm checks and coverage (1 + 5 platform jobs) |
| `.github/workflows/reflow-release-version.yml` | Reusable workflow for version extraction and tag verification |
| `.github/workflows/reflow-release-build.yml` | Reusable workflow for release binary builds (5 platforms) |
| `.github/workflows/reflow-release-npm.yml` | Reusable workflow for npm package, publish, and test |
| `.github/workflows/cleanup-npm-staging.yml` | Scheduled: unpublish staging packages older than 2.2 days |
| `.github/workflows/update-openapi-spec.yml` | Nightly/manual OpenAPI spec update, creates PR (planned) |

## Concurrency

The top-level `ci.yml` workflow uses a concurrency group to cancel redundant PR runs. When a new commit is pushed to a PR, any in-progress CI run for that same PR is automatically cancelled.

| Event | Group key | Behavior |
| ----- | --------- | -------- |
| `pull_request` | `workflow_ref` + PR number | New push cancels previous run |
| `push` (main) | `run_id` (unique) | Runs are never cancelled |
| `push` (tag) | `run_id` (unique) | Runs are never cancelled |

The concurrency group is only set on `ci.yml`. Reusable workflows (`reflow-*.yml`) inherit cancellation from the caller — when `ci.yml` is cancelled, all its called workflows are cancelled along with it.

## CI Architecture

The CI uses reusable workflows for visual grouping in the GitHub Actions UI. Each reusable workflow appears as a collapsible group containing its matrix jobs.

```text
┌──────────────────────────────────────────────────────────────────┐
│           ci.yml (entry point for CI and releases)                │
├──────────────────────────────────────────────────────────────────┤
│  triggers: push (main, v* tags), pull_request                     │
│                                                                   │
│  jobs:                                                            │
│    pre-commit:                                                    │
│      uses: ./.github/workflows/reflow-pre-commit.yml             │
│                                                                   │
│    rust:                                                          │
│      uses: ./.github/workflows/reflow-rust.yml                   │
│                                                                   │
│    coverage:                                                      │
│      uses: ./.github/workflows/reflow-coverage.yml               │
│                                                                   │
│    npm:                                                           │
│      uses: ./.github/workflows/reflow-npm.yml                    │
│                                                                   │
│    version:                                                       │
│      uses: ./.github/workflows/reflow-release-version.yml        │
│      (tag verification when triggered by tag push)                │
│                                                                   │
│    build:                                                         │
│      uses: ./.github/workflows/reflow-release-build.yml          │
│                                                                   │
│    checks: (needs: pre-commit, rust, coverage, npm)               │
│      re-actors/alls-green — gates release jobs on all quality     │
│      checks passing                                               │
│                                                                   │
│    release-config: (needs: version)                               │
│      Centralizes all release-mode decisions:                      │
│        tag push → publish=true, real version, latest dist-tag     │
│        otherwise → publish=false, staging version (tarballs only) │
│                                                                   │
│    release-npm: (needs: release-config, version, build, checks)   │
│      uses: ./.github/workflows/reflow-release-npm.yml            │
│      (version and dist-tag from release-config)                   │
│                                                                   │
│    cargo-publish: (needs: release-config, checks)                 │
│      cargo package --workspace (always)                           │
│      cargo publish in dependency order (tag push only)            │
│                                                                   │
│    publish-release: (needs: release-config, version, build,       │
│                     release-npm, cargo-publish)                    │
│      uses: reflow-publish-release.yml                             │
│                                                                   │
│    tag-release: (needs: checks, main only)                        │
│      uses: reflow-tag-release.yml                                 │
│                                                                   │
│    post-release: (needs: tag-release, main only, if tagged)       │
│      uses: reflow-post-release.yml                                │
│                                                                   │
│    all:                                                           │
│      needs: [checks, tag-release, post-release,                  │
│              release-npm, cargo-publish, publish-release]         │
│      allowed-skips: tag-release, post-release                     │
│      uses: re-actors/alls-green                                  │
└──────────────────────────────────────────────────────────────────┘

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
│    resolve-versions: resolves rustup/Docker Hub versions    │
│    lint: 1 job (fmt, clippy, deny on stable)                │
│    build: 15 jobs (3 rust × 5 platforms)                    │
│      - Builds and archives tests with cargo-nextest         │
│      - Linux builds target musl for static linking          │
│    test: 21 jobs (3 rust × 7 platform/libc combinations)    │
│      - Runs archived tests                                  │
│      - Linux tests on both glibc and musl                   │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│         reflow-coverage.yml (reusable workflow)              │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    library: uses reflow-library.yml                         │
│    check: 5 platform jobs (cargo-llvm-cov, stable only)     │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│            reflow-npm.yml (reusable workflow)                │
├─────────────────────────────────────────────────────────────┤
│  jobs:                                                       │
│    library: uses reflow-library.yml                         │
│    coverage: 5 platform jobs (c8 coverage)                  │
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

**Note:** Each reusable workflow calls `reflow-library.yml` internally to get the matrix configuration. This results in the library job running 6 times per CI run (once per reusable workflow that needs platform matrices), but the overhead is minimal.

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
      # ...
    - name: Windows
      # ...
  arch:
    - name: ARM
      matrix: arm
      emoji: 💪
    - name: Intel
      matrix: intel
      emoji: 🌀
  rust:
    - name: MSRV
      matrix: msrv
      version: "1.89"
      emoji: ⏬
    - name: stable
      matrix: stable
      version: stable
      emoji: 🪨
    - name: beta
      matrix: beta
      version: beta
      emoji: 🔮
  libc:
    - name: native
      matrix: native
      emoji: 🏠
    - name: glibc
      matrix: glibc
      emoji: 🐃
    - name: musl
      matrix: musl
      emoji: 🦌

exclude:
  # For jobs without libc axis (coverage, pre-commit, build)
  windows-arm:
    - os: { matrix: windows }
      arch: { matrix: arm }

  # For test job with libc axis
  test:
    - os: { matrix: windows }
      arch: { matrix: arm }
    - os: { matrix: macos }
      libc: { matrix: glibc }
    - os: { matrix: macos }
      libc: { matrix: musl }
    - os: { matrix: windows }
      libc: { matrix: glibc }
    - os: { matrix: windows }
      libc: { matrix: musl }
    - os: { matrix: linux }
      libc: { matrix: native }
```

The action parses this YAML and outputs it as JSON for use in workflow matrices.

**Libc axis:** Used by the test job to run Linux tests on both glibc (Ubuntu) and musl (Alpine)
environments, verifying that statically-linked musl binaries work correctly on glibc systems.

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
| MSRV (1.89) | Yes | From `rust-toolchain.toml` |
| Latest stable | Yes | Primary development target |
| Beta | No | Allowed to fail |

### Job Count

| Workflow | Jobs |
| -------- | ---- |
| Pre-commit | 6 (1 library + 5 check) |
| Rust | 39 (1 library + 1 resolve-versions + 1 lint + 15 build + 21 test) |
| Coverage | 6 (1 library + 5 check) |
| npm | 6 (1 library + 5 coverage) |
| Version | 1 |
| Build | 6 (1 library + 5 build) |
| Checks | 1 (alls-green gate) |
| Release config | 1 |
| Release npm | 13 (1 library + 1 package + 5 test-tarballs + 1 publish + 5 test-published) |
| Cargo publish | 1 |
| GitHub Release | 1 |
| All | 1 |
| **Total** | **82 jobs** |

**Rust job breakdown:**

- **Lint:** 1 job (runs fmt, clippy, deny on stable)
- **Build:** 15 jobs (3 rust versions × 5 platforms)
- **Test:** 21 jobs (3 rust versions × 7 platform/libc combinations)
  - Linux: 2 arch × 2 libc (glibc + musl) = 4 combinations
  - macOS: 2 arch × 1 (native) = 2 combinations
  - Windows: 1 arch × 1 (native) = 1 combination
  - Total: 7 combinations × 3 rust versions = 21 jobs

## CI Tooling

| Tool | Purpose |
| ------ | --------- |
| [Mergify](https://mergify.com) | Merge queue, autoqueue, CI Insights auto-retry |
| [mergifyio/gha-mergify-ci](https://github.com/mergifyio/gha-mergify-ci) | Upload JUnit test results to Mergify CI Insights |
| [pre-commit/action](https://github.com/pre-commit/action) | Runs pre-commit hooks in CI |
| [re-actors/alls-green](https://github.com/re-actors/alls-green) | Aggregate job status |
| [actions-rust-lang/setup-rust-toolchain](https://github.com/actions-rust-lang/setup-rust-toolchain) | Rust toolchain installation |
| [taiki-e/install-action](https://github.com/taiki-e/install-action) | Install cargo tools (cargo-deny, cargo-llvm-cov, cargo-nextest) |
| [cargo-nextest](https://nexte.st/) | Next-generation test runner with archiving support |
| [c8](https://github.com/bcoe/c8) | V8 native coverage for Node.js (npm wrapper) |
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

The Rust workflow is split into lint, build, and test stages:

| Stage | Check | Tool | Run On |
| ----- | ----- | ---- | ------ |
| Lint | Formatting | `cargo fmt --check` | 1 job (stable) |
| Lint | Linting | `cargo clippy` | 1 job (stable) |
| Lint | Dependency audit | `cargo deny` | 1 job (stable) |
| Build | Compile + archive | `cargo nextest archive` | 15 jobs (3 rust × 5 platforms) |
| Test | Run tests | `cargo-nextest run` | 21 jobs (3 rust × 7 platform/libc) |

**Build/Test split rationale:** Linux binaries are built targeting musl (Rust provides
built-in support with self-contained linking), then tested on both glibc (Ubuntu) and
musl (Alpine) environments to verify portability.

### Coverage

#### Rust Coverage

| Check | Tool | Run On |
| ------- | ------ | -------- |
| Coverage | `cargo-llvm-cov` | Stable only, all 5 platforms |
| Upload | `codecov/codecov-action` | With OIDC authentication, `rust` flag |

#### npm Coverage

| Check | Tool | Run On |
| ------- | ------ | -------- |
| Coverage | `c8` (V8 native coverage) | Node.js 24, all 5 platforms |
| Upload | `codecov/codecov-action` | With OIDC authentication, `npm` flag |

Both Rust and npm coverage use Codecov flags to separate the coverage data. The `codecov.yml` configuration defines components for each, enabling per-component coverage tracking in PR comments.

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
| Location | `crates/onshape-mcp-io/onshape-openapi.json` |
| Source | `https://cad.onshape.com/api/v6/openapi` |
| License | Apache 2.0 (see `crates/onshape-mcp-io/ONSHAPE-API-LICENSE`) |
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
