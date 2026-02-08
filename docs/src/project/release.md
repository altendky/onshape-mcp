# Release

## Distribution

Users can install the server via:

| Method | Description |
| -------- | ------------- |
| `npx onshape-mcp` | Via npm wrapper package (see [npm Wrapper](npm-wrapper.md)) |
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
| `cargo-publish` job | Publish workspace crates to crates.io (or `--dry-run` validation) |
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
| Publish npm | Yes (`--tag staging`) | Yes (`--tag latest`) |
| Test npm from registry | Yes | Yes |
| Validate crate packaging | `cargo package --workspace` | `cargo package --workspace` |
| `cargo publish` | — | Yes (real publish) |
| Package release archives | Yes | Yes |
| Generate SHA256SUMS | Yes | Yes |
| Create GitHub release | — | Yes |

## Version Strategy

### Source of Truth

The authoritative version is in `crates/onshape-mcp/Cargo.toml`.
All npm `package.json` files are synced to this version via `scripts/sync-npm-versions.js`, enforced by a pre-commit hook.

### Staging Version Format

Staging publishes use a pre-release version to ensure uniqueness and traceability:

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
To prevent this for staging publishes, pass `--tag <name>`.

For staging publishes, a single `staging` dist-tag is used (`npm publish --tag staging`).
This prevents `latest` from moving while keeping things simple — packages are installed by exact version string, not by dist-tag.
The `staging` tag always points to the most recent staging publish.
For the finalize publish, the `latest` tag moves to the real version (the default `npm publish` behavior).

## Version Workflow (`reflow-release-version.yml`)

Extracts the version from `crates/onshape-mcp/Cargo.toml`, verifies npm packages are in sync, and optionally verifies the version matches a git tag.

- Input: `git-tag` (optional)
- Output: `version`
- Verify all npm `package.json` versions match Cargo.toml (via `scripts/sync-npm-versions.js --check`)
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
- `npm pack` all 6 packages
- Upload as `npm-tarballs` artifact

**2. test-tarballs** (matrix: 5 platforms, needs: package)

- Install from tarballs in temp directory
- Run `npx onshape-mcp --version`, verify output matches `binary-version`

**3. publish** (ubuntu-latest, needs: test-tarballs)

- Skip if no npm credentials available (fork PRs)
- `npm publish --tag {dist-tag} <tarball>` for 5 platform packages first
- `npm publish --tag {dist-tag} <main-tarball>` last

**4. test-published** (matrix: 5 platforms, needs: publish)

- Skip if publish was skipped
- `npm install onshape-mcp@{version}` in temp directory
- Run `npx onshape-mcp --version`, verify output

Staging packages are **not** cleaned up by the npm workflow.
They remain on npm for up to 2.2 days (52.8 hours), allowing manual testing of PR builds.
Cleanup is handled by a separate scheduled workflow — see [Staging Cleanup](#staging-cleanup).

## Staging Cleanup

Staging npm packages are cleaned up by a **scheduled workflow** (`cleanup-npm-staging.yml`), not by the staging workflow itself.
This allows manual testing of PR builds for a window of time after the CI run completes.

| Setting | Value |
| ------- | ----- |
| Schedule | Every 6 hours (`cron: '0 */6 * * *'`) |
| Max age | 2.2 days (52.8 hours) |
| npm unpublish deadline | 72 hours (3 days) |
| Worst-case buffer | 13.2 hours (52.8h max age + 6h interval = 58.8h, vs 72h deadline) |

The workflow:

1. Lists all versions of `onshape-mcp` and each `@onshape-mcp/*` platform package
2. Filters for staging versions (matching the `*-staging-*` pattern)
3. Checks publish timestamps for each staging version
4. Unpublishes any staging version older than 2.2 days (52.8 hours)
5. Unpublishes main package before platform packages (reverse of publish order)

Staging versions are identifiable by their format: `{version}-staging-{sanitized_ref}-{commit_sha}-{run_id}`.

## Release Pipeline

Releases are triggered by pushing a `v*` tag to the repository. This triggers the same `ci.yml` workflow that runs on PRs, but the `release-config` job detects the tag push and switches all downstream jobs to publish mode.

### Release Job Flow

```text
version ──► release-config ──┬──► release-npm ◄── build
                              │         │
                              ├──► cargo-publish
                              │         │
                              └──► github-release
                                   (needs: release-config, version,
                                    build, release-npm, cargo-publish)

build starts immediately (no dependency on version or release-config)
```

`build` and `version` start immediately in parallel (neither has dependencies). `release-config` depends on `version` and makes a single mode decision — all other release jobs consume its outputs. `cargo-publish` and `release-npm` depend on `release-config`; `release-npm` also depends on `build`. `github-release` waits for everything.

### Release-Config Job

The `release-config` job is the single point where `github.ref_type == 'tag'` is evaluated. It outputs:

| Output | Tag push (publish) | PR / branch push (validate) |
| ------ | ------------------ | --------------------------- |
| `publish` | `true` | `false` |
| `npm-version` | Cargo.toml version | Staging version |
| `npm-dist-tag` | `latest` | `staging` |

### Inline Jobs

**cargo-publish** (ubuntu-latest, needs: release-config)

- Always validates crate packaging via `cargo package --workspace`, which checks Cargo.toml metadata, file inclusion, dependency resolution (using sibling `.crate` files for workspace deps), and builds from the packaged source
- On tag push: publishes all workspace crates in dependency order (see [Crate Naming and Publish Order](#crate-naming-and-publish-order))

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
| `npm-tarballs` (6) | `reflow-release-npm.yml` package job | npm test-tarballs, npm publish, npm test-published |

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
| `.github/workflows/cleanup-npm-staging.yml` | Scheduled: unpublish staging packages older than 2.2 days (52.8 hours) |
| `.github/scripts/compute-staging-version.sh` | Computes staging version with sanitized ref, commit SHA, run ID |
| `.github/scripts/cargo-publish-workspace.sh` | Publishes all workspace crates to crates.io in dependency order |

## Modified Files

| File | Change |
| ---- | ------ |
| `.github/workflows/ci.yml` | Unified CI and release entry point: version, build, release-config, release-npm, cargo-publish, github-release jobs; `all` gate covers everything |
| `docs/src/project/ci.md` | Document unified pipeline and updated job counts |

## Secrets and Permissions

| Secret | Used by | Purpose |
| ------ | ------- | ------- |
| `NPM_TOKEN` | npm publish (fallback) + cleanup | npm publish (fork PRs), npm unpublish (cleanup) |
| `CARGO_REGISTRY_TOKEN` | `ci.yml` cargo-publish job | crates.io publish (not needed for `--dry-run`) |
| `GITHUB_TOKEN` | `ci.yml` github-release job | GitHub release (automatic, not a manual secret) |

| Permission | Job | Purpose |
| ---------- | --- | ------- |
| `id-token: write` | `release-npm` | npm OIDC trusted publishing, provenance |
| `contents: write` | `github-release` | GitHub release creation |

### npm Authentication Strategy

npm publish uses OIDC trusted publishing for both CI and release:

| Context | Auth method | Mechanism |
| ------- | ----------- | --------- |
| CI staging + release | OIDC trusted publishing | `id-token: write` on `release-npm` job; npm CLI auto-detects |
| Fork PRs | `NPM_TOKEN` secret (fallback) | OIDC unavailable; token in `~/.npmrc` |
| `cleanup-npm-staging.yml` | `NPM_TOKEN` secret | `npm unpublish` does not support OIDC |

npm's trusted publishing allows only **one workflow filename per package**.
The trusted publisher is configured for `ci.yml` on npmjs.com, which handles both CI and release.
The publish job in `reflow-release-npm.yml` detects which credentials are available (`ACTIONS_ID_TOKEN_REQUEST_URL` for OIDC, `NPM_TOKEN` for token) and skips if neither is present (fork PRs).

Provenance attestations are generated automatically when publishing via OIDC trusted publishing (no `--provenance` flag needed).

## Open Items

Items requiring further discussion before or during implementation.

### Checksums and Signing

- [x] Checksums: ~~confirm approach~~ Resolved: SHA-256 with a single `SHA256SUMS` file in `sha256sum` output format, covering all GitHub release assets. Compatible with `sha256sum --check` (Linux) and `shasum -a 256 --check` (macOS)
- [ ] Sigstore/cosign signing — deferred. Would add cryptographic provenance verification (proving artifacts were built by CI, not just integrity via checksums). Uses GitHub Actions OIDC for keyless signing — no key management needed. Can be layered on later by adding a `.bundle` file to GitHub release assets. See [sigstore.dev](https://sigstore.dev)

### crates.io

- [x] Scope: ~~publish all crates vs just the binary~~ Resolved: publish all workspace crates. The binary crate depends on the library crates, so `cargo install onshape-mcp` requires them on crates.io. See [Crate Naming and Publish Order](#crate-naming-and-publish-order)
- [x] Grouping: ~~investigate whether crates.io has grouping or organization abilities~~ Resolved: crates.io has no namespace, organization, or grouping mechanism. All crate names share a flat global namespace. Ownership can be shared via GitHub teams (`cargo owner --add github:org:team`), but each crate is managed independently. Grouping is by naming convention only (`onshape-mcp-*` prefix)
- [x] Auth: ~~traditional token or OIDC~~ Resolved: use `CARGO_REGISTRY_TOKEN` for now. crates.io supports OIDC trusted publishing (fully replaces tokens, no secret to manage) but deferred to keep initial implementation simple

### npm

- [x] Auth: ~~traditional token or OIDC~~ Resolved: OIDC trusted publishing for both CI and release. npm allows one workflow filename per package; `ci.yml` is configured as the trusted publisher on npmjs.com. `NPM_TOKEN` serves as a fallback for fork PRs and for `cleanup-npm-staging.yml`. Provenance attestations are generated automatically via OIDC. See [npm Authentication Strategy](#npm-authentication-strategy)
- [x] Dist-tag: ~~unique per publish vs single reusable~~ Resolved: use a single `--tag staging` for all staging publishes. Packages are installed by exact version (`npm install onshape-mcp@0.2.0-staging-main-abc1234-12345678`), so the dist-tag is only needed to prevent `latest` from moving. The `staging` tag always points to the most recent staging publish, which is mildly useful for quick testing (`npm install onshape-mcp@staging`)
- [x] Staging cleanup strategy: ~~per-run vs scheduled~~ Resolved: separate scheduled workflow (`cleanup-npm-staging.yml`) runs every 6 hours, unpublishes staging packages older than 2.2 days (52.8 hours). This preserves staging packages for manual testing of PR builds while staying within npm's 72-hour unpublish window (worst case: 58.8 hours, 13.2-hour buffer). See [Staging Cleanup](#staging-cleanup)
- [x] Cleanup order: ~~main first or platform packages first~~ Resolved: unpublish main package first, then platform packages (reverse of publish order). The main package has `optionalDependencies` on the platform packages, so removing main first avoids broken dependency resolution during the cleanup window
- [x] Platform `package.json` files: ~~add `"files": ["bin"]`~~ Resolved: added to all 5 platform packages, ensuring only the `bin/` directory is included in published tarballs
- [x] Staging vs release npm tarballs: ~~confirm approach~~ Resolved: the `reflow-release-npm.yml` workflow is parameterized by version. Each invocation packs once with the provided version — CI passes a staging version, release passes the real version. No double-packing needed

### CI Integration

- [x] CI gate: ~~blocking or non-blocking~~ Resolved: `release-npm`, `cargo-publish`, and `github-release` are all required by the `all` gate in `ci.yml`, blocking merge if any fail

### GitHub Release

- [x] Naming convention: ~~human-friendly vs Rust target triples~~ Resolved: use Rust target triples with version included, distributed as tarballs (`.tar.gz` for Unix, `.zip` for Windows). Example: `onshape-mcp-0.2.0-x86_64-unknown-linux-musl.tar.gz`. Tarballs preserve file permissions (execute bit), can include license files, and match the Rust ecosystem convention (ripgrep, bat, starship). Each tarball contains the binary plus `LICENSE-MIT` and `LICENSE-APACHE`

### Workflow Inputs

- [x] `workflow_dispatch` inputs: ~~determine whether to accept parameters~~ Resolved: removed. Release is triggered only by tag push. The unified `ci.yml` pipeline exercises the full release flow on every PR — including `cargo publish --dry-run`, archive packaging, and SHA256SUMS generation — providing comprehensive coverage without a manual dispatch trigger. See [#88](https://github.com/altendky/onshape-mcp/issues/88)
