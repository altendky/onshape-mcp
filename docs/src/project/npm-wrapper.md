# npm Wrapper Package

This document describes the npm wrapper package design that enables installation via `npx --yes onshape-mcp`.

## Overview

The npm distribution uses a multi-package architecture with platform-specific binaries distributed as optional dependencies.
This pattern (used by projects like `swc` and `esbuild`) provides:

- Pre-built binaries (no Rust toolchain required)
- Automatic platform detection
- Small download size (only the relevant binary is installed)
- Fallback instructions for unsupported platforms

## Package Structure

| Package | Description |
| ------- | ----------- |
| `onshape-mcp` | Main package with JS shim and platform detection |
| `@onshape-mcp/linux-x64` | Linux x86_64 binary |
| `@onshape-mcp/linux-arm64` | Linux ARM64 binary |
| `@onshape-mcp/darwin-x64` | macOS x86_64 binary |
| `@onshape-mcp/darwin-arm64` | macOS ARM64 (Apple Silicon) binary |
| `@onshape-mcp/win32-x64` | Windows x86_64 binary |

### Linux Static Linking

Linux binaries are compiled with the `x86_64-unknown-linux-musl` (or `aarch64-unknown-linux-musl`) target, producing fully static binaries.
This ensures compatibility with both glibc-based distributions (Ubuntu, Debian, Fedora) and musl-based distributions (Alpine).

## Directory Structure

```text
npm/
├── onshape-mcp/              # Main package
│   ├── package.json
│   ├── bin.js                # JS shim entry point
│   └── README.md
├── linux-x64/                # Platform packages
│   ├── package.json
│   └── bin/
│       └── onshape-mcp
├── linux-arm64/
│   ├── package.json
│   └── bin/
│       └── onshape-mcp
├── darwin-x64/
│   ├── package.json
│   └── bin/
│       └── onshape-mcp
├── darwin-arm64/
│   ├── package.json
│   └── bin/
│       └── onshape-mcp
└── win32-x64/
    ├── package.json
    └── bin/
        └── onshape-mcp.exe
```

## How It Works

### Optional Dependencies

The published `onshape-mcp` package declares all platform packages as `optionalDependencies`.
npm only installs the optional dependency matching the current platform, keeping installation fast and lightweight.

**Publish-time injection:** The source `package.json` does **not** contain `optionalDependencies`.
They are injected by the CI publish workflow (`reflow-release-npm.yml`) at packaging time using `jq`.
This avoids lockfile degradation during version bumps — since unpublished platform packages
at the new version can't be resolved by `npm install`, keeping them out of the source
`package.json` means the lockfile is always accurate.

The published package looks like (example showing illustrative version 0.1.0):

```json
{
  "name": "onshape-mcp",
  "optionalDependencies": {
    "@onshape-mcp/linux-x64": "0.1.0",
    "@onshape-mcp/linux-arm64": "0.1.0",
    "@onshape-mcp/darwin-x64": "0.1.0",
    "@onshape-mcp/darwin-arm64": "0.1.0",
    "@onshape-mcp/win32-x64": "0.1.0"
  }
}
```

### JavaScript Shim

The `bin.js` shim handles platform detection and binary execution:

1. Detects the current platform (`process.platform`) and architecture (`process.arch`)
2. Locates the corresponding platform package binary
3. Spawns the binary with inherited stdio
4. Exits with the binary's exit code

### Signal Handling

The synchronous execution model (`execFileSync` with `stdio: 'inherit'`) handles OS signals naturally without manual forwarding.

**Unix (Linux, macOS):**

- Both the JS shim and Rust binary share the same controlling terminal
- They belong to the same foreground process group
- When the user sends SIGINT (Ctrl+C) or SIGTERM, the kernel delivers the signal to the entire process group
- The Rust binary receives the signal directly and handles graceful shutdown
- No manual signal forwarding is required in the JS shim

**Windows:**

- Windows uses console control events instead of Unix signals
- With inherited stdio, console events (like Ctrl+C) reach the child process directly

**Exit code propagation:**

```js
try {
  execFileSync(binPath, process.argv.slice(2), { stdio: 'inherit' });
} catch (e) {
  process.exit(e.status ?? 1);
}
```

### stdio Behavior

With `stdio: 'inherit'`, the Rust binary directly inherits the parent's file descriptors.
The JS shim is not in the data path — the MCP client communicates directly with the Rust binary's stdin/stdout.
This avoids any buffering concerns at the JS layer.

For Rust-side stdio buffering considerations, see [#37](https://github.com/altendky/onshape-mcp/issues/37).

### Platform Detection

| `process.platform` | `process.arch` | Package |
| ------------------ | -------------- | ------- |
| `linux` | `x64` | `@onshape-mcp/linux-x64` |
| `linux` | `arm64` | `@onshape-mcp/linux-arm64` |
| `darwin` | `x64` | `@onshape-mcp/darwin-x64` |
| `darwin` | `arm64` | `@onshape-mcp/darwin-arm64` |
| `win32` | `x64` | `@onshape-mcp/win32-x64` |

## Versioning Strategy

All npm packages use **lockstep versioning** with the Cargo version:

- Cargo.toml version: `0.1.0`
- All npm packages: `0.1.0`

This ensures consistency and simplifies the release process.

### Enforcement

Version synchronization is enforced through two mechanisms:

1. **Release script** — Updates all `package.json` files to match the version in `Cargo.toml`
2. **CI check** — Validates that all versions match, failing the build if any mismatch is detected

This defense-in-depth approach prevents version drift from reaching production.

## Fallback Behavior

When running on an unsupported platform (e.g., Windows ARM64, FreeBSD):

1. The shim detects no matching platform package is available
2. Prints a clear error message with the detected platform/architecture
3. Provides instructions for building from source via `cargo install onshape-mcp`
4. Exits with a non-zero status code

Example error output:

```text
Error: Unsupported platform: freebsd-x64

No pre-built binary is available for your platform.
You can build from source using Cargo:

    cargo install onshape-mcp

This requires the Rust toolchain. See https://rustup.rs for installation.
```

## Publishing Workflow

The npm packages are published as part of the release process:

1. **Sync versions** — Run release script to update all `package.json` versions from `Cargo.toml`
2. **Build binaries** — CI builds release binaries for all supported platforms
3. **Validate versions** — CI job verifies all `package.json` versions match `Cargo.toml`
4. **Prepare packages** — Copy binaries into their respective `npm/` platform directories
5. **Inject optionalDependencies** — CI injects the `optionalDependencies` block into the main `package.json` with `jq`
6. **Publish platform packages** — Publish each `@onshape-mcp/*` package to npm
7. **Publish main package** — Publish the main `onshape-mcp` package last

The main package must be published last to ensure all platform dependencies are available when users install it.

### Version Validation

The CI version check will:

1. Extract version from `Cargo.toml`
2. Compare against each `package.json` in `npm/`
3. Fail with clear error message if any mismatch is found

This runs on every PR to catch version drift before merge.

## Testing Strategy

### Pre-publish Verification

- Verify each platform package contains the correct binary
- Verify binaries are executable (`--version` or `--help`)
- Verify the JS shim correctly locates and spawns the binary
- Test fallback error message on unsupported platforms

### CI Integration

- Run `npm pack` + install for each platform in CI
- Smoke test on native platform runners (Linux x64, Linux ARM64, macOS x64, macOS ARM64, Windows x64)
- ARM testing uses actual ARM runners, not emulation

### Coverage

The JS shim (`bin.js`) has coverage monitoring using `c8` (V8 native coverage):

```bash
# Run tests with coverage
npm run test:coverage

# Run tests without coverage
npm test
```

Coverage runs on all 5 platforms in CI and uploads to Codecov with the `npm` flag. See [CI > Coverage](ci.md#coverage) for details.

### End-to-end Testing

- Test `npx --yes onshape-mcp` against an MCP client
- Verify JSON-RPC communication over stdio
- Verify graceful shutdown on SIGINT/SIGTERM

## Usage

Once published, users can run the server directly:

```bash
npx --yes onshape-mcp
```

Or install globally:

```bash
npm install -g onshape-mcp
onshape-mcp
```

## Local Development

To test the npm wrapper with a locally-built Rust binary:

1. **Build the Rust binary:**

   ```bash
   cargo build          # Debug build
   cargo build --release  # Release build
   ```

2. **Run the npm wrapper:**

   ```bash
   node npm/onshape-mcp/bin.js --help
   ```

3. **Or enable in opencode:**
   Set `"enabled": true` for `onshape-npm-debug` or `onshape-npm-release` in `opencode.json`.

### Repository Detection

When running from the repository, the JS shim (`bin.js`) automatically detects the local development environment by checking for `Cargo.toml` at the repository root.
It then looks for the binary in `target/debug/` first, falling back to `target/release/`.

### Environment Variables

| Variable | Description |
| -------- | ----------- |
| `ONSHAPE_MCP_NPM_COMMAND` | Override the binary command (shell-quoted). Bypasses auto-detection. |

#### Examples

```bash
# Use a specific binary path
ONSHAPE_MCP_NPM_COMMAND="/path/to/onshape-mcp"

# Use a path with spaces
ONSHAPE_MCP_NPM_COMMAND='"/path with spaces/onshape-mcp"'

# Use a release build explicitly
ONSHAPE_MCP_NPM_COMMAND="./target/release/onshape-mcp"

# Run via another interpreter (for testing)
ONSHAPE_MCP_NPM_COMMAND="node /path/to/mock-binary.js"

# Add prefix arguments
ONSHAPE_MCP_NPM_COMMAND="strace -f /path/to/onshape-mcp"
```

The command is parsed using shell quoting rules (via the `shell-quote` package), so paths with spaces must be quoted.
Shell operators like pipes (`|`), redirects (`>`), and background (`&`) are not supported and will produce an error.

The `opencode.json` entries for `onshape-npm-debug` and `onshape-npm-release` use this variable to select the appropriate build.
