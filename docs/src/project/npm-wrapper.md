# npm Wrapper Package

This document describes the npm wrapper package design that enables installation via `npx onshape-mcp`.

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

The main `onshape-mcp` package declares all platform packages as `optionalDependencies`:

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

npm only installs the optional dependency matching the current platform, keeping installation fast and lightweight.

### JavaScript Shim

The `bin.js` shim handles platform detection and binary execution:

1. Detects the current platform (`process.platform`) and architecture (`process.arch`)
2. Locates the corresponding platform package binary
3. Spawns the binary with inherited stdio
4. Exits with the binary's exit code

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

1. **Build binaries** — CI builds release binaries for all supported platforms
2. **Prepare packages** — Copy binaries into their respective `npm/` platform directories
3. **Update versions** — Ensure all `package.json` files have the release version
4. **Publish platform packages** — Publish each `@onshape-mcp/*` package to npm
5. **Publish main package** — Publish the main `onshape-mcp` package last

The main package must be published last to ensure all platform dependencies are available when users install it.

## Usage

Once published, users can run the server directly:

```bash
npx onshape-mcp
```

Or install globally:

```bash
npm install -g onshape-mcp
onshape-mcp
```
