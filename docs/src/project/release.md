# Release

## Distribution

Users can install the server via:

| Method | Description |
| -------- | ------------- |
| `npx onshape-mcp` | Via npm wrapper package (see [npm Wrapper](npm-wrapper.md)) |
| `cargo install` | From crates.io |
| Pre-built binaries | GitHub releases for all supported platforms |

## Workflow Architecture

The release pipeline is split into two reusable workflows:

| Workflow | Purpose |
| -------- | ------- |
| `reflow-release-staging.yml` | Build, test, npm staging publish, verify, cleanup |
| `reflow-release-finalize.yml` | Publish real versions to npm/crates.io, create GitHub release |

The staging workflow always publishes a **unique pre-release version** (never the real version).
The finalize workflow is the only path that publishes the real version.
This eliminates idempotency concerns — there is no scenario where two runs compete for the same npm version.

### Calling Patterns

| Caller | Trigger | Staging | Finalize |
| ------ | ------- | ------- | -------- |
| `ci.yml` | PR / push to main | Yes | No |
| `release.yml` | `workflow_dispatch` | Yes | No |
| `release.yml` | Tag push (`v*`) | Yes | Yes (after staging succeeds) |

### Trigger Behavior

| Step | PR / push to main | Manual dispatch | Git tag push |
| ---- | ----------------- | --------------- | ------------ |
| Verify tag == Cargo.toml | — | — | Yes (fail if mismatch) |
| Build release binaries | Yes | Yes | Yes |
| Smoke test binaries | Yes | Yes | Yes |
| Package npm tarballs | Yes | Yes | Yes |
| Test npm from tarballs | Yes | Yes | Yes |
| Publish npm staging | Yes (pre-release version) | Yes (pre-release version) | Yes (pre-release version) |
| Test npm from registry | Yes | Yes | Yes |
| Cleanup npm staging | Yes (`always`) | Yes (`always`) | Yes (`always`) |
| `cargo publish --dry-run` | Yes | Yes | — |
| `cargo publish` | — | — | Yes |
| Publish npm `latest` | — | — | Yes |
| Create GitHub release | — | — | Yes |

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
- Manual dispatch on `main`: `0.2.0-staging-main-abc1234-12345678`

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

## Staging Workflow (`reflow-release-staging.yml`)

### Job Flow

```text
verify ──► build (5 platforms) ──► package-npm ──► test-npm-tarballs (5 platforms)
                                       │                     │
                                       │                     ▼
                                       │          publish-npm-staging ──► test-npm-published (5 platforms)
                                       │                                          │
                                       │                                          ▼
                                       │                               cleanup-npm-staging
                                       │                                   (if: always)
                                       ▼
                              [artifacts for finalize]
```

### Jobs

**1. verify** (ubuntu-latest)

- Extract version from `crates/onshape-mcp/Cargo.toml`
- If `git-tag` input provided: strip `v` prefix, compare to Cargo.toml version, fail on mismatch
- Run `node scripts/sync-npm-versions.js --check`
- Compute staging version: `{version}-staging-{sanitized_ref}-{commit_sha}-{run_id}`
- `cargo package -p onshape-mcp --no-verify` — produces `.crate` file
- `cargo publish -p onshape-mcp --dry-run`
- Upload `.crate` as artifact
- Outputs: `version`, `staging-version`

**2. build** (matrix: 5 platforms)

| Runner | Rust target | npm platform |
| ------ | ----------- | ------------ |
| `ubuntu-latest` | `x86_64-unknown-linux-musl` | `linux-x64` |
| `ubuntu-24.04-arm` | `aarch64-unknown-linux-musl` | `linux-arm64` |
| `macos-15-intel` | (native) | `darwin-x64` |
| `macos-latest` | (native) | `darwin-arm64` |
| `windows-latest` | (native) | `win32-x64` |

- `cargo build --release` (with musl target on Linux)
- Linux: verify static linking via `.github/scripts/verify-static-linking.sh`
- Smoke test: `./target/release/onshape-mcp --version`
- Upload binary as artifact `binary-{platform}`

**3. package-npm** (ubuntu-latest, needs: build + verify)

- Download all 5 binary artifacts
- Copy each into `npm/{platform}/bin/onshape-mcp` (`.exe` for win32)
- Pack with **real version** (from Cargo.toml): `npm pack` all 6 packages — upload as `npm-release-tarballs`
- Update all `package.json` to staging version
- Pack with **staging version**: `npm pack` all 6 packages — upload as `npm-staging-tarballs`
- Generate `SHA256SUMS` for all release artifacts (binaries, release tarballs, `.crate` file) — upload as artifact

Two sets of tarballs are needed because the version string is baked into `package.json` inside each tarball.
The binaries inside are identical — only the `package.json` version field differs.

**4. test-npm-tarballs** (matrix: 5 platforms, needs: package-npm)

- Download staging tarballs
- Install in temp directory from tarballs
- Run `npx onshape-mcp --version`, verify output matches Cargo version

**5. publish-npm-staging** (ubuntu-latest, needs: test-npm-tarballs)

- Skip if `NPM_TOKEN` secret unavailable (fork PRs)
- `npm publish --tag staging <tarball>` for 5 platform packages first
- `npm publish --tag staging <main-tarball>` last (main package must be published after platform packages)

**6. test-npm-published** (matrix: 5 platforms, needs: publish-npm-staging)

- Skip if publish was skipped
- `npm install onshape-mcp@{staging-version}` in temp directory
- Run `npx onshape-mcp --version`, verify output

Staging packages are **not** cleaned up by this workflow.
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

## Finalize Workflow (`reflow-release-finalize.yml`)

Called only on tag push, after staging succeeds.
Downloads artifacts produced by the staging workflow — no rebuilding.

### Finalize Job Flow

```text
publish-npm ──► publish-crates ──► github-release
```

### Finalize Jobs

**1. publish-npm** (ubuntu-latest)

- Download `npm-release-tarballs` artifact
- `npm publish <tarball>` for 5 platform packages (gets `latest` tag by default)
- `npm publish <main-tarball>` last

**2. publish-crates** (ubuntu-latest, needs: publish-npm)

- `cargo publish` for all workspace crates in dependency order (see [Crate Naming and Publish Order](#crate-naming-and-publish-order))

**3. github-release** (ubuntu-latest, needs: publish-crates)

- Download all artifacts: binaries, release npm tarballs, `.crate` file, `SHA256SUMS`
- Create GitHub release from tag via `gh release create`
- Upload all artifacts to the release

## Artifacts

All publishable artifacts are created in the staging workflow and passed to the finalize workflow via GitHub Actions upload/download.

| Artifact | Created by | Reusable across staging/release? | Consumed by |
| -------- | ---------- | -------------------------------- | ----------- |
| Platform binaries (5) | `build` job | Yes — binary embeds Cargo version, not npm version | `package-npm`, finalize `github-release` |
| npm staging tarballs (6) | `package-npm` job | No — staging version baked into `package.json` | `test-npm-tarballs`, `publish-npm-staging` |
| npm release tarballs (6) | `package-npm` job | N/A — these are the real-version tarballs | finalize `publish-npm`, finalize `github-release` |
| `.crate` file | `verify` job | Yes — uses real version from Cargo.toml | finalize `publish-crates`, finalize `github-release` |
| `SHA256SUMS` | `package-npm` job | Yes | finalize `github-release` |

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

The `SHA256SUMS` file covers all assets uploaded to the GitHub release (binaries, npm tarballs, `.crate` file).
It is created in the staging workflow alongside the release artifacts.

## New Files

| File | Purpose |
| ---- | ------- |
| `.github/workflows/release.yml` | Entry point for manual dispatch + tag push |
| `.github/workflows/reflow-release-staging.yml` | Reusable: build, test, staging publish |
| `.github/workflows/reflow-release-finalize.yml` | Reusable: real publish, crates.io, GitHub release |
| `.github/workflows/cleanup-npm-staging.yml` | Scheduled: unpublish staging packages older than 2.2 days (52.8 hours) |
| `.github/scripts/compute-staging-version.sh` | Computes staging version with sanitized ref, commit SHA, run ID |

## Modified Files

| File | Change |
| ---- | ------ |
| `.github/workflows/ci.yml` | Add call to `reflow-release-staging.yml`, wire into `all` gate |
| `docs/src/project/ci.md` | Document new workflows and updated job counts |

## Secrets and Permissions

| Secret | Used by | Purpose |
| ------ | ------- | ------- |
| `NPM_TOKEN` | staging + finalize | npm publish |
| `CARGO_REGISTRY_TOKEN` | finalize | crates.io publish |
| `GITHUB_TOKEN` | finalize | GitHub release (automatic, not a manual secret) |

| Permission | Workflow | Purpose |
| ---------- | -------- | ------- |
| `id-token: write` | staging + finalize | npm provenance (future) |
| `contents: write` | finalize | GitHub release creation |

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

- [x] Auth: ~~traditional token or OIDC~~ Resolved: use `NPM_TOKEN` (required regardless — npm does not support OIDC as a token replacement). npm provenance (`--provenance`, OIDC-signed build attestation) deferred
- [x] Dist-tag: ~~unique per publish vs single reusable~~ Resolved: use a single `--tag staging` for all staging publishes. Packages are installed by exact version (`npm install onshape-mcp@0.2.0-staging-main-abc1234-12345678`), so the dist-tag is only needed to prevent `latest` from moving. The `staging` tag always points to the most recent staging publish, which is mildly useful for quick testing (`npm install onshape-mcp@staging`)
- [x] Staging cleanup strategy: ~~per-run vs scheduled~~ Resolved: separate scheduled workflow (`cleanup-npm-staging.yml`) runs every 6 hours, unpublishes staging packages older than 2.2 days (52.8 hours). This preserves staging packages for manual testing of PR builds while staying within npm's 72-hour unpublish window (worst case: 58.8 hours, 13.2-hour buffer). See [Staging Cleanup](#staging-cleanup)
- [x] Cleanup order: ~~main first or platform packages first~~ Resolved: unpublish main package first, then platform packages (reverse of publish order). The main package has `optionalDependencies` on the platform packages, so removing main first avoids broken dependency resolution during the cleanup window
- [x] Platform `package.json` files: ~~add `"files": ["bin"]`~~ Resolved: added to all 5 platform packages, ensuring only the `bin/` directory is included in published tarballs
- [x] Staging vs release npm tarballs: ~~confirm approach~~ Resolved: the `package-npm` job creates two sets of tarballs. First, pack with the real version (from Cargo.toml) and upload as release artifacts for the finalize workflow. Then, update `package.json` versions to the staging version and pack again for staging publish/test. The binaries inside are identical — only the `package.json` version field differs. This is necessary because the version is baked into `package.json` inside each tarball

### CI Integration

- [x] CI gate: ~~blocking or non-blocking~~ Resolved: `release-staging` is required by the `all` gate in `ci.yml`, blocking merge if it fails

### GitHub Release

- [x] Naming convention: ~~human-friendly vs Rust target triples~~ Resolved: use Rust target triples with version included, distributed as tarballs (`.tar.gz` for Unix, `.zip` for Windows). Example: `onshape-mcp-0.2.0-x86_64-unknown-linux-musl.tar.gz`. Tarballs preserve file permissions (execute bit), can include license files, and match the Rust ecosystem convention (ripgrep, bat, starship). Each tarball contains the binary plus `LICENSE-MIT` and `LICENSE-APACHE`

### Workflow Inputs

- [x] `workflow_dispatch` inputs: ~~determine whether to accept parameters~~ Resolved: two optional inputs. **`dry-run`** (boolean, default false) skips `npm publish` while running the full build/test pipeline. **`skip-cleanup`** (boolean, default false) preserves staging packages on npm for manual inspection. Branch selection is handled by GitHub's built-in UI when triggering manually; version is always read from Cargo.toml
