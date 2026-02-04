"use strict";

const { describe, it } = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const BIN_JS = path.join(__dirname, "..", "bin.js");
const LIB_JS = path.join(__dirname, "..", "lib.js");
const MOCK_BINARY = path.join(__dirname, "fixtures", "mock-binary.js");

// Import exported functions for unit testing
const {
  PLATFORMS,
  getPlatformPackage,
  getBinaryPath,
  parseCommand,
} = require("../lib.js");

// Check if a local development binary exists (same logic as bin.js)
function localBinaryExists() {
  const repoRoot = path.join(__dirname, "..", "..", "..");
  if (!fs.existsSync(path.join(repoRoot, "Cargo.toml"))) {
    return false;
  }
  const ext = process.platform === "win32" ? ".exe" : "";
  const binName = `onshape-mcp${ext}`;
  return (
    fs.existsSync(path.join(repoRoot, "target", "debug", binName)) ||
    fs.existsSync(path.join(repoRoot, "target", "release", binName))
  );
}

// Helper to run bin.js with specific env vars
function runBin(args = [], envOverrides = {}) {
  // Start with minimal env to avoid interference
  const env = {
    PATH: process.env.PATH,
    NODE_PATH: process.env.NODE_PATH,
    ...envOverrides,
  };

  return spawnSync(process.execPath, [BIN_JS, ...args], {
    encoding: "utf8",
    env,
  });
}

// Helper to run bin.js with mock binary via ONSHAPE_MCP_NPM_COMMAND
function runWithMock(args = [], commandOverride = null) {
  const command = commandOverride ?? `${process.execPath} ${MOCK_BINARY}`;
  return runBin(args, { ONSHAPE_MCP_NPM_COMMAND: command });
}

// Helper to parse mock binary JSON output
function parseMockOutput(result) {
  assert.strictEqual(
    result.status,
    0,
    `Expected exit 0, got ${result.status}. stderr: ${result.stderr}`
  );
  return JSON.parse(result.stdout);
}

describe("bin.js", () => {
  describe("platform detection", () => {
    it("should have mappings for all supported platforms", () => {
      const expectedPlatforms = [
        "linux-x64",
        "linux-arm64",
        "darwin-x64",
        "darwin-arm64",
        "win32-x64",
      ];

      // Read the lib.js file and verify PLATFORMS object contains all expected keys
      const libContent = fs.readFileSync(LIB_JS, "utf8");

      for (const platform of expectedPlatforms) {
        assert.ok(
          libContent.includes(`"${platform}"`),
          `Platform ${platform} should be defined in PLATFORMS`
        );
      }
    });
  });

  describe("getPlatformPackage()", () => {
    it("should return package name for current platform if supported", () => {
      const key = `${process.platform}-${process.arch}`;
      const result = getPlatformPackage();

      if (PLATFORMS[key]) {
        assert.strictEqual(result, PLATFORMS[key]);
      } else {
        assert.strictEqual(result, null);
      }
    });

    it("should have correct package names in PLATFORMS map", () => {
      assert.strictEqual(PLATFORMS["linux-x64"], "@onshape-mcp/linux-x64");
      assert.strictEqual(PLATFORMS["linux-arm64"], "@onshape-mcp/linux-arm64");
      assert.strictEqual(PLATFORMS["darwin-x64"], "@onshape-mcp/darwin-x64");
      assert.strictEqual(PLATFORMS["darwin-arm64"], "@onshape-mcp/darwin-arm64");
      assert.strictEqual(PLATFORMS["win32-x64"], "@onshape-mcp/win32-x64");
    });

    it("should not have mapping for unsupported platforms", () => {
      // These platform/arch combinations are not supported
      assert.strictEqual(PLATFORMS["freebsd-x64"], undefined);
      assert.strictEqual(PLATFORMS["linux-arm"], undefined);
      assert.strictEqual(PLATFORMS["win32-arm64"], undefined);
    });
  });

  describe("getBinaryPath()", () => {
    it("should return a string or null", () => {
      const result = getBinaryPath();

      assert.ok(
        result === null || typeof result === "string",
        `Expected null or string, got ${typeof result}`
      );
    });

    it("should return existing file path when binary is found", () => {
      const result = getBinaryPath();

      if (result !== null) {
        assert.ok(
          fs.existsSync(result),
          `Binary path should exist: ${result}`
        );
      }
    });

    it("should return path with correct binary name", () => {
      const result = getBinaryPath();

      if (result !== null) {
        const expectedExt = process.platform === "win32" ? ".exe" : "";
        const expectedName = `onshape-mcp${expectedExt}`;
        assert.ok(
          result.endsWith(expectedName),
          `Binary path should end with ${expectedName}, got: ${result}`
        );
      }
    });

    it("should check local development paths when in repo root", () => {
      const result = getBinaryPath();
      const repoRoot = path.join(__dirname, "..", "..", "..");

      // If we're in a repo with Cargo.toml, binary should be from target/
      if (fs.existsSync(path.join(repoRoot, "Cargo.toml")) && result !== null) {
        assert.ok(
          result.includes("target"),
          `In dev environment, binary should be from target/, got: ${result}`
        );
      }
    });

    it("should prefer debug binary over release in development", () => {
      const result = getBinaryPath();
      const repoRoot = path.join(__dirname, "..", "..", "..");
      const ext = process.platform === "win32" ? ".exe" : "";
      const binName = `onshape-mcp${ext}`;
      const debugBin = path.join(repoRoot, "target", "debug", binName);
      const releaseBin = path.join(repoRoot, "target", "release", binName);

      // If both debug and release exist, should return debug
      if (fs.existsSync(debugBin) && fs.existsSync(releaseBin)) {
        assert.strictEqual(result, debugBin, "Should prefer debug over release");
      }
    });

    it("should fall back to release binary when debug not available", () => {
      const repoRoot = path.join(__dirname, "..", "..", "..");
      const ext = process.platform === "win32" ? ".exe" : "";
      const binName = `onshape-mcp${ext}`;
      const debugBin = path.join(repoRoot, "target", "debug", binName);
      const releaseBin = path.join(repoRoot, "target", "release", binName);

      // This test documents the expected behavior
      // If only release exists (debug doesn't), should return release
      if (!fs.existsSync(debugBin) && fs.existsSync(releaseBin)) {
        const result = getBinaryPath();
        assert.strictEqual(result, releaseBin, "Should use release when debug unavailable");
      }
    });
  });

  describe("parseCommand()", () => {
    it("should parse simple command", () => {
      const result = parseCommand("/path/to/binary");
      assert.deepStrictEqual(result, ["/path/to/binary"]);
    });

    it("should parse command with arguments", () => {
      const result = parseCommand("/path/to/binary --arg1 --arg2");
      assert.deepStrictEqual(result, ["/path/to/binary", "--arg1", "--arg2"]);
    });

    it("should parse command with quoted path", () => {
      const result = parseCommand('"/path with spaces/binary"');
      assert.deepStrictEqual(result, ["/path with spaces/binary"]);
    });

    it("should parse command with quoted arguments", () => {
      const result = parseCommand('cmd "arg with spaces" --flag');
      assert.deepStrictEqual(result, ["cmd", "arg with spaces", "--flag"]);
    });

    it("should throw on empty command", () => {
      assert.throws(
        () => parseCommand(""),
        /Command is empty after parsing/
      );
    });

    it("should throw on whitespace-only command", () => {
      assert.throws(
        () => parseCommand("   "),
        /Command is empty after parsing/
      );
    });

    it("should throw on pipe operator", () => {
      assert.throws(
        () => parseCommand("cmd1 | cmd2"),
        /Unsupported shell operator/
      );
    });

    it("should throw on redirect operator", () => {
      assert.throws(
        () => parseCommand("cmd > file.txt"),
        /Unsupported shell operator/
      );
    });

    it("should throw on background operator", () => {
      assert.throws(
        () => parseCommand("cmd &"),
        /Unsupported shell operator/
      );
    });

    it("should throw on semicolon operator", () => {
      assert.throws(
        () => parseCommand("cmd1 ; cmd2"),
        /Unsupported shell operator/
      );
    });

    it("should throw on && operator", () => {
      assert.throws(
        () => parseCommand("cmd1 && cmd2"),
        /Unsupported shell operator/
      );
    });

    it("should throw on || operator", () => {
      assert.throws(
        () => parseCommand("cmd1 || cmd2"),
        /Unsupported shell operator/
      );
    });
  });

  describe("binary detection", () => {
    // Test that bin.js can find and run the actual binary (when it exists)
    const skip = !localBinaryExists();

    it("should find and run local binary without ONSHAPE_MCP_NPM_COMMAND", { skip }, () => {
      // Run bin.js without env var - should find local binary and show help
      const result = runBin(["--help"]);

      // Should succeed (or at least not fail with "unsupported platform")
      const stderr = result.stderr || "";
      assert.ok(
        !stderr.includes("Unsupported platform"),
        `Should find local binary, got stderr: ${stderr}`
      );
    });
  });

  describe("unsupported platform error", () => {
    // These tests only run when no local binary exists
    // (e.g., in CI smoke tests before Rust build)
    const skip = localBinaryExists();

    it("should provide helpful error message when binary not found", { skip }, () => {
      // Run bin.js without ONSHAPE_MCP_NPM_COMMAND - should fail gracefully
      const result = runBin();

      // Should exit with non-zero status
      assert.notStrictEqual(result.status, 0, "Should exit with error");

      // Error message should be helpful
      const stderr = result.stderr || "";
      assert.ok(
        stderr.includes("cargo install onshape-mcp") ||
          stderr.includes("Unsupported platform"),
        `Should provide helpful error message, got: ${stderr}`
      );
    });

    it("should mention rustup.rs for toolchain installation", { skip }, () => {
      const result = runBin();

      const stderr = result.stderr || "";
      // When binary not found, should mention rustup
      if (stderr.includes("Unsupported platform")) {
        assert.ok(
          stderr.includes("rustup.rs"),
          "Should mention rustup.rs for Rust installation"
        );
      }
    });
  });

  describe("ONSHAPE_MCP_NPM_COMMAND", () => {
    describe("command execution", () => {
      it("should execute command from env var", () => {
        const result = runWithMock();
        const output = parseMockOutput(result);

        assert.ok(Array.isArray(output.args), "Mock should receive args array");
      });

      it("should work with quoted paths containing spaces", () => {
        // Use shell quoting for path (even though this path doesn't have spaces,
        // it tests the quoting mechanism)
        const command = `"${process.execPath}" "${MOCK_BINARY}"`;
        const result = runWithMock([], command);
        const output = parseMockOutput(result);

        assert.ok(Array.isArray(output.args), "Mock should receive args array");
      });
    });

    describe("argument passthrough", () => {
      it("should pass simple arguments", () => {
        const result = runWithMock(["--version"]);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["--version"]);
      });

      it("should pass multiple arguments", () => {
        const result = runWithMock(["--help", "--verbose", "file.txt"]);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["--help", "--verbose", "file.txt"]);
      });

      it("should pass arguments with equals sign", () => {
        const result = runWithMock(["--config=/path/to/config"]);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["--config=/path/to/config"]);
      });

      it("should pass arguments with spaces when quoted at shell level", () => {
        // Note: The argument reaches Node.js already parsed by the OS
        // This tests that bin.js passes it through unchanged
        const result = runWithMock(["arg with spaces"]);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["arg with spaces"]);
      });

      it("should pass arguments with special characters", () => {
        const result = runWithMock(["--pattern=*.txt", "$HOME", "file@host"]);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["--pattern=*.txt", "$HOME", "file@host"]);
      });

      it("should handle empty arguments list", () => {
        const result = runWithMock([]);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, []);
      });

      it("should handle many arguments", () => {
        const manyArgs = Array.from({ length: 100 }, (_, i) => `arg${i}`);
        const result = runWithMock(manyArgs);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, manyArgs);
      });
    });

    describe("command prefix arguments", () => {
      it("should combine command prefix args with user args", () => {
        // Command has extra args that should come before user args
        const command = `${process.execPath} ${MOCK_BINARY} --prefix-arg`;
        const result = runWithMock(["--user-arg"], command);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["--prefix-arg", "--user-arg"]);
      });

      it("should handle multiple prefix args", () => {
        const command = `${process.execPath} ${MOCK_BINARY} --first --second`;
        const result = runWithMock(["--third"], command);
        const output = parseMockOutput(result);

        assert.deepStrictEqual(output.args, ["--first", "--second", "--third"]);
      });
    });

    describe("exit code propagation", () => {
      it("should propagate exit code 0", () => {
        const result = runWithMock(["--exit", "0"]);

        assert.strictEqual(result.status, 0);
      });

      it("should propagate exit code 1", () => {
        const result = runWithMock(["--exit", "1"]);

        assert.strictEqual(result.status, 1);
      });

      it("should propagate exit code 2", () => {
        const result = runWithMock(["--exit", "2"]);

        assert.strictEqual(result.status, 2);
      });

      it("should propagate exit code 127", () => {
        const result = runWithMock(["--exit", "127"]);

        assert.strictEqual(result.status, 127);
      });
    });

    describe("parse errors", () => {
      it("should error on command with shell operators", () => {
        const result = runBin([], { ONSHAPE_MCP_NPM_COMMAND: "cmd1 | cmd2" });

        assert.notStrictEqual(result.status, 0, "Should exit with error");
        assert.ok(
          result.stderr.includes("Failed to parse ONSHAPE_MCP_NPM_COMMAND") ||
            result.stderr.includes("Unsupported shell operator"),
          `Should mention parse failure or unsupported operator, got: ${result.stderr}`
        );
      });

      it("should error on command with redirect operators", () => {
        const result = runBin([], { ONSHAPE_MCP_NPM_COMMAND: "cmd > file.txt" });

        assert.notStrictEqual(result.status, 0, "Should exit with error");
        assert.ok(
          result.stderr.includes("Failed to parse ONSHAPE_MCP_NPM_COMMAND") ||
            result.stderr.includes("Unsupported shell operator"),
          `Should mention parse failure or unsupported operator, got: ${result.stderr}`
        );
      });

      it("should error on command with background operator", () => {
        const result = runBin([], { ONSHAPE_MCP_NPM_COMMAND: "cmd &" });

        assert.notStrictEqual(result.status, 0, "Should exit with error");
        assert.ok(
          result.stderr.includes("Failed to parse ONSHAPE_MCP_NPM_COMMAND") ||
            result.stderr.includes("Unsupported shell operator"),
          `Should mention parse failure or unsupported operator, got: ${result.stderr}`
        );
      });

      it("should provide helpful examples in error message", () => {
        const result = runBin([], { ONSHAPE_MCP_NPM_COMMAND: "cmd | other" });

        assert.ok(
          result.stderr.includes("ONSHAPE_MCP_NPM_COMMAND="),
          `Should provide example usage, got: ${result.stderr}`
        );
      });

      // Note: Empty string and whitespace-only values are falsy in JS,
      // so they fall back to auto-detection rather than erroring.
      // This is intentional - setting VAR="" is often used to "unset" a variable.
    });
  });
});
