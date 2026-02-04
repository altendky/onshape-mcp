"use strict";

const { execFileSync } = require("node:child_process");
const { existsSync } = require("node:fs");
const { join, dirname } = require("node:path");
const { parse } = require("shell-quote");

const PLATFORMS = {
  "linux-x64": "@onshape-mcp/linux-x64",
  "linux-arm64": "@onshape-mcp/linux-arm64",
  "darwin-x64": "@onshape-mcp/darwin-x64",
  "darwin-arm64": "@onshape-mcp/darwin-arm64",
  "win32-x64": "@onshape-mcp/win32-x64",
};

function getPlatformPackage() {
  const key = `${process.platform}-${process.arch}`;
  return PLATFORMS[key] || null;
}

function getBinaryPath(baseDir = __dirname) {
  const pkg = getPlatformPackage();
  const ext = process.platform === "win32" ? ".exe" : "";
  const binName = `onshape-mcp${ext}`;

  // 1. Try npm-installed package (production path)
  if (pkg) {
    try {
      const pkgJsonPath = require.resolve(`${pkg}/package.json`);
      const binPath = join(dirname(pkgJsonPath), "bin", binName);
      if (existsSync(binPath)) {
        return binPath;
      }
    } catch {
      // Package not installed via npm
    }
  }

  // 2. Local development: check target/ directory
  //    Try debug first, then release
  const repoRoot = join(baseDir, "../..");
  if (existsSync(join(repoRoot, "Cargo.toml"))) {
    const debugBin = join(repoRoot, "target", "debug", binName);
    if (existsSync(debugBin)) {
      return debugBin;
    }
    const releaseBin = join(repoRoot, "target", "release", binName);
    if (existsSync(releaseBin)) {
      return releaseBin;
    }
  }

  return null;
}

function parseCommand(commandStr) {
  const parsed = parse(commandStr);

  // shell-quote can return objects for operators like { op: '|' }
  // We only accept plain string tokens
  for (const token of parsed) {
    if (typeof token !== "string") {
      throw new Error(
        `Unsupported shell operator in command: ${JSON.stringify(token)}`
      );
    }
  }

  if (parsed.length === 0) {
    throw new Error("Command is empty after parsing");
  }

  return parsed;
}

function main() {
  let exe;
  let prefixArgs = [];

  const commandEnv = process.env.ONSHAPE_MCP_NPM_COMMAND;
  if (commandEnv) {
    try {
      const parsed = parseCommand(commandEnv);
      [exe, ...prefixArgs] = parsed;
    } catch (error) {
      process.stderr.write(
        `Error: Failed to parse ONSHAPE_MCP_NPM_COMMAND: ${error.message}

The ONSHAPE_MCP_NPM_COMMAND environment variable should contain a shell-quoted command.
Examples:
    ONSHAPE_MCP_NPM_COMMAND="/path/to/onshape-mcp"
    ONSHAPE_MCP_NPM_COMMAND="node /path/to/script.js"
    ONSHAPE_MCP_NPM_COMMAND='"/path with spaces/onshape-mcp"'
`
      );
      process.exit(1);
    }
  } else {
    exe = getBinaryPath();
  }

  if (!exe) {
    const platform = `${process.platform}-${process.arch}`;
    process.stderr.write(`Error: Unsupported platform: ${platform}

No pre-built binary is available for your platform.
You can build from source using Cargo:

    cargo install onshape-mcp

This requires the Rust toolchain. See https://rustup.rs for installation.
`);
    process.exit(1);
  }

  const args = [...prefixArgs, ...process.argv.slice(2)];

  try {
    execFileSync(exe, args, { stdio: "inherit" });
  } catch (error) {
    process.exit(error.status ?? 1);
  }
}

module.exports = {
  PLATFORMS,
  getPlatformPackage,
  getBinaryPath,
  parseCommand,
  main,
};
