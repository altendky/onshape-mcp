# Release

## Distribution

Users can install the server via:

| Method | Description |
| -------- | ------------- |
| `npx --yes onshape-mcp` | Via npm wrapper package (see [npm Wrapper](npm-wrapper.md)) |
| `cargo install` | From crates.io |
| Pre-built binaries | GitHub releases for all supported platforms |

## Workflow Architecture

CI and release share a single unified pipeline in `ci.yml`. A `release-config` job inspects the trigger (tag push vs PR/branch push) and centralizes all mode-dependent decisions. Downstream jobs consume named outputs and have no conditional logic of their own.

The pipeline is composed of three reusable workflows plus two inline jobs:

| Component | Purpose |
| --------- | ------- |
| `reflow-release-version.yml` | Extract version from Cargo.toml, verify tag match |
| `reflow-release-build.yml` | Build release binaries on 5 platforms |
| `reflow-release-npm.yml` | Package, publish, and test npm packages (parameterized by version and dist-tag) |
| `cargo-publish` job | Publish workspace crates to crates.io (or `cargo package` validation) |
| `github-release` job | Package archives, generate SHA256SUMS, create GitHub Release |

### Trigger Behavior

All steps run on every trigger. The `release-config` job determines whether each step performs a real publish or a validation-only pass:

| Step | PR / push to main | Git tag push |
| ---- | ----------------- | ------------ |
| Verify tag == Cargo.toml | — | Yes (fail if mismatch) |
| Build release binaries | Yes | Yes |
| Smoke test binaries | Yes | Yes |
| Package npm tarballs | Yes (staging version) | Yes (real version) |
| Test npm from tarballs | Yes | Yes |
| Publish npm | — | Yes (`--tag latest`) |
| Test npm from registry | — | Yes |
| Validate crate packaging | `cargo package --workspace` | `cargo package --workspace` |
| `cargo publish` | — | Yes (real publish) |
| Package release archives | Yes | Yes |
| Generate SHA256SUMS | Yes | Yes |
| Create GitHub release | — | Yes |

## Version Strategy

### Source of Truth

The authoritative version is `[workspace.package].version` in the root `Cargo.toml`.
All crate `Cargo.toml` files inherit this version via `version.workspace = true`.
The `[workspace.dependencies]` internal crate version entries, `Cargo.lock`, and all npm `package.json` files are synced to this version via `scripts/sync-versions.js`, enforced by a pre-commit hook.

### Staging Version Format

Non-tag CI runs use a pre-release version for npm tarball naming to ensure uniqueness and traceability:

```text
{version}-staging-{sanitized_ref}-{commit_sha}-{run_id}
```

Examples:

- PR branch `feature/add-auth`: `0.2.0-staging-feature-add-auth-abc1234-12345678`
- Tag `v0.2.0`: `0.2.0-staging-v0-2-0-abc1234-12345678`
- Push to `main`: `0.2.0-staging-main-abc1234-12345678`

### Sanitization

Branch and tag names are sanitized for semver pre-release identifier compatibility.
Only `[0-9A-Za-z-]` is allowed; invalid characters (e.g., `/`, `.`) are replaced with `-`.

## npm Concepts: Versions vs Dist-tags

npm has two distinct mechanisms that are easy to conflate:

| Concept | Example | Mutable? | Purpose |
| ------- | ------- | -------- | ------- |
| **Version** | `0.2.0-staging-main-abc1234-12345678` | No — permanent once published | Identity of the package |
| **Dist-tag** | `latest`, `staging` | Yes — a mutable pointer to a version | Convenience label for `npm install` |

When you run `npm publish`, npm automatically moves the `latest` dist-tag to point to the new version.
To prevent this, pass `--tag <name>` to publish under a different dist-tag instead.

Currently, only tag pushes publish to npm (using the default `latest` dist-tag).
Staging versions are computed for CI tarball naming but are not published to the registry.
The `staging` dist-tag and `cleanup-npm-staging.yml` workflow are retained for potential future use with test release paths.

## Version Workflow (`reflow-release-version.yml`)

Extracts the version from the workspace `Cargo.toml`, verifies all versions are in sync, and optionally verifies the version matches a git tag.

- Input: `git-tag` (optional)
- Output: `version`
- Verify all versions are in sync (via `scripts/sync-versions.js --check`)
- If `git-tag` provided: strip `v` prefix, compare to Cargo.toml version, fail on mismatch

## Build Workflow (`reflow-release-build.yml`)

Builds release binaries on all 5 platforms.

| Runner | Rust target | npm platform |
| ------ | ----------- | ------------ |
| `ubuntu-latest` | `x86_64-unknown-linux-musl` | `linux-x64` |
| `ubuntu-24.04-arm` | `aarch64-unknown-linux-musl` | `linux-arm64` |
| `macos-15-intel` | (native) | `darwin-x64` |
| `macos-latest` | (native) | `darwin-arm64` |
| `windows-latest` | (native) | `win32-x64` |

- `cargo build --release` (with musl target on Linux)
- Linux: verify static linking
- Smoke test: `./target/release/onshape-mcp --version`
- Upload binary as artifact `binary-{platform}`

## npm Workflow (`reflow-release-npm.yml`)

Packages, publishes, and tests npm packages. Parameterized by version and dist-tag.

### Inputs

| Input | Description | Example |
| ----- | ----------- | ------- |
| `version` | Version to set in `package.json` | `0.1.0` or `0.1.0-staging-main-abc1234-12345678` |
| `binary-version` | Expected `--version` output | `0.1.0` (always the Cargo.toml version) |
| `dist-tag` | npm dist-tag for publish | `staging` or `latest` |

### Job Flow

```text
package ──► test-tarballs (5 platforms) ──► publish ──► test-published (5 platforms)
```

### Jobs

**1. package** (ubuntu-latest)

- Download all 5 binary artifacts (from the build workflow)
- Copy each into `npm/{platform}/bin/onshape-mcp` (`.exe` for win32)
- Update all `package.json` to the input `version`
- `npm pack` all 7 packages (5 platform + opencode-auth plugin + main)
- Upload as `npm-tarballs` artifact

**2. test-tarballs** (matrix: 5 platforms, needs: package)

- Install from tarballs in temp directory
- Run `npx --yes onshape-mcp --version`, verify output matches `binary-version`

**3. publish** (ubuntu-latest, needs: test-tarballs)

- Skip if no npm credentials available (fork PRs)
- `npm publish --tag {dist-tag} <tarball>` for 5 platform packages first
- `npm publish --tag {dist-tag} <plugin-tarball>` for opencode-auth plugin
- `npm publish --tag {dist-tag} <main-tarball>` last

**4. test-published** (matrix: 5 platforms, needs: publish)

- Skip if publish was skipped
- `npm install onshape-mcp@{version}` in temp directory
- Run `npx --yes onshape-mcp --version`, verify output

Staging versions are only used for tarball naming in CI — they are not published to npm.
Only tag pushes (real releases) publish to the npm registry.

## Staging Versions

Staging versions (e.g., `0.2.0-staging-main-abc1234-12345678`) are computed on non-tag CI runs for use in npm tarball naming only.
They are **not published** to the npm registry — only tag pushes trigger real npm publishes.
The staging version format provides traceability in CI artifacts (branch, commit, run ID) without generating external notifications.

## Release Automation

The release process is PR-driven: a human runs a mise task to create a release-prep PR, and after merge, automation handles tagging, publishing, and the post-release version bump.

### Version Scheme

| State | Version | Example |
| ----- | ------- | ------- |
| Release | `X.Y.Z` | `0.5.0` |
| Between releases | `X.Y.(Z+1)-dev.0` | `0.5.1-dev.0` |

Semver pre-release format, valid for Cargo.toml, crates.io, and npm. The `-` in the version is the detection mechanism for `release.yml` (contains `-` = not a release commit). Post-release bump is always patch. If the next release is minor or major, the `release` mise task specifies the target version explicitly.

### Release Task (`mise run release`)

Creates a release-prep PR:

```bash
mise run release 0.5.0
```

Steps:

1. Validates version format (`X.Y.Z`, no pre-release suffix)
2. Verifies clean working tree on `main`
3. Pulls latest and checks tag doesn't already exist
4. Creates branch `release/v{version}`
5. Updates `[workspace.package].version` in root `Cargo.toml`
6. Runs `node scripts/sync-versions.js` (propagates to workspace deps, Cargo.lock, all npm packages)
7. Commits, pushes, opens PR with `enqueue` label

### Auto-Tag Workflow (`reflow-tag-release.yml`)

A reusable workflow called from `ci.yml` as the `tag-release` job, gated on `needs: [checks]`. This ensures the merge commit passes all quality checks before a tag is created. Uses the `altendky-release` GitHub App token (via `actions/create-github-app-token`) so that tag pushes trigger `ci.yml` (pushes with `GITHUB_TOKEN` do not trigger workflows).

The `tag-release` job only runs on pushes to `main` (`if: github.ref == 'refs/heads/main'`).

Steps:

1. Read version from `Cargo.toml` (via `cargo metadata`)
2. If version contains `-` → exit (pre-release / dev version, not a release commit)
3. If tag `v{version}` already exists → exit (idempotent)
4. Create and push tag `v{version}` (triggers `ci.yml` publish pipeline)
5. Compute next patch dev version (e.g., `0.5.0` → `0.5.1-dev.0`)
6. Create branch `post-release/v{version}`, update version, run `sync-versions.js`
7. Open post-release PR with `enqueue` label

### Mergify Auto-Approve

Post-release PRs from `altendky-release[bot]` on `post-release/*` branches are automatically approved. The existing merge queue handles merge (requires `enqueue` label + 1 approval).

### Release Flow

```text
mise run release 0.5.0
        │
        ▼
  release/v0.5.0 PR ──merge──► push to main
                                    │
                             release.yml triggers
                                    │
                             ┌──────┴──────┐
                             │  create tag  │
                             │  v0.5.0      │
                             └──────┬──────┘
                                    │
                       ┌────────────┴────────────┐
                       │                         │
                       ▼                         ▼
                ci.yml (tag)              post-release/
                publishes to:             v0.5.0 PR
                - crates.io              (0.5.1-dev.0)
                - npm                         │
                - GitHub Release              ▼
                                        auto-approve
                                        + merge
                                              │
                                       push to main
                                              │
                                       release.yml triggers
                                       version has `-`
                                       → exit (no-op)
```

## Release Pipeline

The publish pipeline lives in `ci.yml` and is triggered by pushing a `v*` tag. The `tag-release` job in `ci.yml` (calling `reflow-tag-release.yml`) automates tag creation after a release-prep PR merges, gated on all quality checks passing (see [Release Automation](#release-automation)). The `release-config` job detects the tag push and switches all downstream jobs to publish mode.

### Release Job Flow

```text
pre-commit ─┐
rust ───────┤
coverage ───┼──► checks (alls-green)
npm ────────┘         │
                      ├──► cargo-publish
                      │         │
version ──► release-config ──┤  │
                      │      │  │
build ────────────────┼──► release-npm
                      │         │
                      └──► github-release
                           (needs: release-config, version,
                            build, release-npm, cargo-publish)
```

All quality jobs (`pre-commit`, `rust`, `coverage`, `npm`) must pass the `checks` gate before any publishing can begin. `build` and `version` start immediately in parallel. `release-config` depends on `version` and makes a single mode decision. `cargo-publish` and `release-npm` both depend on `release-config` and `checks`; `release-npm` also depends on `build`. `github-release` waits for everything.

### Release-Config Job

The `release-config` job is the single point where `github.ref_type == 'tag'` is evaluated. It outputs:

| Output | Tag push (publish) | PR / branch push (validate) |
| ------ | ------------------ | --------------------------- |
| `publish` | `true` | `false` |
| `npm-version` | Cargo.toml version | Staging version |
| `npm-dist-tag` | `latest` | `staging` |

### Inline Jobs

**cargo-publish** (ubuntu-latest, needs: release-config + checks)

- Always validates crate packaging via `cargo package --workspace`, which checks Cargo.toml metadata, file inclusion, dependency resolution (using sibling `.crate` files for workspace deps), and builds from the packaged source
- On tag push: publishes all workspace crates in dependency order (see [Crate Naming and Publish Order](#crate-naming-and-publish-order))
- Gated by `checks` — no crate is published unless all quality checks pass

**github-release** (ubuntu-latest, needs: release-config + version + build + release-npm + cargo-publish)

- Download binary artifacts from the build workflow
- Package release archives (tar.gz for Unix, zip for Windows) with license files
- Generate `SHA256SUMS` covering all release archives
- These steps run on every trigger, validating archive packaging in CI
- Create GitHub release from tag via `gh release create` — only when `publish=true`

### GitHub Release Contents

| Asset | Description |
| ----- | ----------- |
| Platform archives (5) | Binary + `LICENSE-MIT` + `LICENSE-APACHE` per platform |
| `SHA256SUMS` | SHA-256 checksums for all archives |

## Artifacts

Artifacts are shared across workflow runs via GitHub Actions upload/download.

| Artifact | Created by | Consumed by |
| -------- | ---------- | ----------- |
| `binary-{platform}` (5) | `reflow-release-build.yml` | `reflow-release-npm.yml`, `github-release` job |
| `npm-tarballs` (7) | `reflow-release-npm.yml` package job | npm test-tarballs, npm publish, npm test-published |

## Crate Naming and Publish Order

All workspace crates are published to crates.io.
The naming follows Rust ecosystem conventions: `-proto` suffix for sans-IO protocol crates (following the pattern established by `quinn-proto`, `hickory-proto`, etc.), with the clean base name for the IO layer.

Crate names are scoped under `onshape-mcp-*` to group them on crates.io (which has no organization or namespace mechanism — all crate names share a flat global namespace).
If the client crates mature into a general-purpose Onshape SDK, they can be renamed to drop the `mcp` scoping.

### Naming Scheme

| Side | Sans-IO | IO |
| ---- | ------- | -- |
| MCP server | `onshape-mcp-proto` | `onshape-mcp` (lib.rs + main.rs) |
| Onshape API | `onshape-mcp-client-proto` | `onshape-mcp-client` |

The MCP server binary and IO library are merged into a single crate (`onshape-mcp`) following the quinn model — `quinn-proto` for the sans-IO layer, `quinn` for the IO layer and public API.
Rust crates can have both `lib.rs` and `main.rs`; the library is usable as a dependency while `cargo install` builds the binary.

The tracing crates are standalone (not Onshape-specific).
See [Open Questions](open-questions.md#tracing-crate-naming) for the naming decision.

### Full Crate List

Publish order follows the dependency chain (leaves first):

| Order | Crate | Current name | Description |
| ----- | ----- | ------------ | ----------- |
| 1 | `tracing-*-macros` | `tracing-sansio-macros` | Proc-macro for tracing capture (name TBD) |
| 2 | `tracing-*` | `tracing-sansio` | Sans-IO tracing capture library (name TBD) |
| 3 | `onshape-mcp-client-proto` | `onshape-client-core` | Sans-IO Onshape API types/logic |
| 4 | `onshape-mcp-client` | (planned) | Onshape API HTTP client |
| 5 | `onshape-mcp-proto` | `onshape-mcp-core` | Sans-IO MCP server protocol logic |
| 6 | `onshape-mcp` | `onshape-mcp` + `onshape-mcp-io` | MCP server IO + binary (merged) |

The exact order may vary as dependencies evolve.
Only crates that exist and are workspace members at the time of release are published.
The rename from current names to target names is a prerequisite for the first release.

## Checksums

Release artifacts are accompanied by a `SHA256SUMS` file using SHA-256.
The file follows the `sha256sum` output format, compatible with `sha256sum --check`:

```text
a1b2c3d4...  onshape-mcp-0.2.0-x86_64-unknown-linux-musl.tar.gz
e5f6a7b8...  onshape-mcp-0.2.0-aarch64-unknown-linux-musl.tar.gz
...
```

The `SHA256SUMS` file covers all assets uploaded to the GitHub release (platform archives).
It is generated in the `github-release` job on every CI run (validating the generation logic) and included in the GitHub Release on tag pushes.

## New Files

| File | Purpose |
| ---- | ------- |
| `.github/workflows/reflow-release-version.yml` | Reusable: extract and validate version |
| `.github/workflows/reflow-release-build.yml` | Reusable: build release binaries on 5 platforms |
| `.github/workflows/reflow-release-npm.yml` | Reusable: package, publish, and test npm packages |
| `.github/workflows/reflow-tag-release.yml` | Reusable: auto-tag on release merge, create post-release version bump PR |
| `.github/workflows/cleanup-npm-staging.yml` | Scheduled: unpublish staging packages older than 2.2 days (52.8 hours) |
| `.github/scripts/compute-staging-version.sh` | Computes staging version with sanitized ref, commit SHA, run ID |
| `.github/scripts/cargo-publish-workspace.sh` | Publishes all workspace crates to crates.io in dependency order |

## Modified Files

| File | Change |
| ---- | ------ |
| `.github/workflows/ci.yml` | Unified CI and release entry point: version, build, release-config, tag-release, release-npm, cargo-publish, github-release jobs; `all` gate covers everything |
| `mise.toml` | Add `release` task for creating release-prep PRs |
| `.mergify.yml` | Auto-approve post-release PRs from `altendky-release[bot]` |
| `docs/src/project/ci.md` | Document unified pipeline and updated job counts |

## Secrets and Permissions

| Secret / Variable | Used by | Purpose |
| ----------------- | ------- | ------- |
| `vars.RELEASE_APP_ID` | `reflow-tag-release.yml` | GitHub App ID for `altendky-release` (repository variable) |
| `secrets.RELEASE_APP_PRIVATE_KEY` | `reflow-tag-release.yml` | GitHub App private key for `altendky-release` |
| `NPM_TOKEN` | npm publish (fallback) + cleanup | npm publish (fork PRs), npm unpublish (cleanup) |
| `GITHUB_TOKEN` | `ci.yml` github-release job | GitHub release (automatic, not a manual secret) |

| Permission | Job | Purpose |
| ---------- | --- | ------- |
| `id-token: write` | `release-npm` | npm OIDC trusted publishing, provenance |
| `id-token: write` | `cargo-publish` | crates.io OIDC trusted publishing |
| `contents: write` | `github-release` | GitHub release creation |

### npm Authentication Strategy

npm publish uses OIDC trusted publishing for releases:

| Context | Auth method | Mechanism |
| ------- | ----------- | --------- |
| Release (tag push) | OIDC trusted publishing | `id-token: write` on `release-npm` job; npm CLI auto-detects |
| Fork PRs | `NPM_TOKEN` secret (fallback) | OIDC unavailable; token in `~/.npmrc` |
| `cleanup-npm-staging.yml` | `NPM_TOKEN` secret | `npm unpublish` does not support OIDC |

npm's trusted publishing allows only **one workflow filename per package**.
The trusted publisher is configured for `ci.yml` on npmjs.com.
The publish job in `reflow-release-npm.yml` only runs on tag pushes and detects which credentials are available (`ACTIONS_ID_TOKEN_REQUEST_URL` for OIDC, `NPM_TOKEN` for token), skipping if neither is present.

Provenance attestations are generated automatically when publishing via OIDC trusted publishing (no `--provenance` flag needed).

## Open Items

Items requiring further discussion before or during implementation.

### Checksums and Signing

- [x] Checksums: ~~confirm approach~~ Resolved: SHA-256 with a single `SHA256SUMS` file in `sha256sum` output format, covering all GitHub release assets. Compatible with `sha256sum --check` (Linux) and `shasum -a 256 --check` (macOS)
- [ ] Sigstore/cosign signing — deferred. Would add cryptographic provenance verification (proving artifacts were built by CI, not just integrity via checksums). Uses GitHub Actions OIDC for keyless signing — no key management needed. Can be layered on later by adding a `.bundle` file to GitHub release assets. See [sigstore.dev](https://sigstore.dev)

### crates.io

- [x] Scope: ~~publish all crates vs just the binary~~ Resolved: publish all workspace crates. The binary crate depends on the library crates, so `cargo install onshape-mcp` requires them on crates.io. See [Crate Naming and Publish Order](#crate-naming-and-publish-order)
- [x] Grouping: ~~investigate whether crates.io has grouping or organization abilities~~ Resolved: crates.io has no namespace, organization, or grouping mechanism. All crate names share a flat global namespace. Ownership can be shared via GitHub teams (`cargo owner --add github:org:team`), but each crate is managed independently. Grouping is by naming convention only (`onshape-mcp-*` prefix)
- [x] Auth: ~~traditional token or OIDC~~ Resolved: OIDC trusted publishing via `rust-lang/crates-io-auth-action`. The workflow filename `ci.yml` is configured as the trusted publisher on crates.io for all workspace crates. No long-lived API tokens required. See [#289](https://github.com/altendky/onshape-mcp/issues/289)

### npm

- [x] Auth: ~~traditional token or OIDC~~ Resolved: OIDC trusted publishing for both CI and release. npm allows one workflow filename per package; `ci.yml` is configured as the trusted publisher on npmjs.com. `NPM_TOKEN` serves as a fallback for fork PRs and for `cleanup-npm-staging.yml`. Provenance attestations are generated automatically via OIDC. See [npm Authentication Strategy](#npm-authentication-strategy)
- [x] Dist-tag: ~~unique per publish vs single reusable~~ Resolved: use a single `--tag staging` for all staging publishes. Packages are installed by exact version (`npm install onshape-mcp@0.2.0-staging-main-abc1234-12345678`), so the dist-tag is only needed to prevent `latest` from moving. The `staging` tag always points to the most recent staging publish, which is mildly useful for quick testing (`npm install onshape-mcp@staging`)
- [x] Staging cleanup strategy: ~~per-run vs scheduled~~ Resolved: staging versions are no longer published to npm ([#89](https://github.com/altendky/onshape-mcp/issues/89)). Only tag pushes publish to npm. Staging versions are used solely for CI tarball naming. See [Staging Versions](#staging-versions)
- [x] Platform `package.json` files: ~~add `"files": ["bin"]`~~ Resolved: added to all 5 platform packages, ensuring only the `bin/` directory is included in published tarballs
- [x] Staging vs release npm tarballs: ~~confirm approach~~ Resolved: the `reflow-release-npm.yml` workflow is parameterized by version. CI passes a staging version for tarball naming (not published), release passes the real version (published). No double-packing needed

### CI Integration

- [x] CI gate: ~~blocking or non-blocking~~ Resolved: `release-npm`, `cargo-publish`, and `github-release` are all required by the `all` gate in `ci.yml`, blocking merge if any fail

### GitHub Release

- [x] Naming convention: ~~human-friendly vs Rust target triples~~ Resolved: use Rust target triples with version included, distributed as tarballs (`.tar.gz` for Unix, `.zip` for Windows). Example: `onshape-mcp-0.2.0-x86_64-unknown-linux-musl.tar.gz`. Tarballs preserve file permissions (execute bit), can include license files, and match the Rust ecosystem convention (ripgrep, bat, starship). Each tarball contains the binary plus `LICENSE-MIT` and `LICENSE-APACHE`

### Workflow Inputs

- [x] `workflow_dispatch` inputs: ~~determine whether to accept parameters~~ Resolved: removed. Release is triggered only by tag push. The unified `ci.yml` pipeline exercises the full release flow on every PR — including `cargo package --workspace`, archive packaging, and SHA256SUMS generation — providing comprehensive coverage without a manual dispatch trigger. See [#88](https://github.com/altendky/onshape-mcp/issues/88)
