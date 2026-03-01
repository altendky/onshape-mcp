#!/usr/bin/env node
"use strict";

/**
 * Syncs all project versions from the workspace version in root Cargo.toml.
 *
 * Propagates [workspace.package].version to:
 *   - [workspace.dependencies] internal crate version entries (root Cargo.toml)
 *   - Cargo.lock (via `cargo update --workspace`)
 *   - All npm package.json files and package-lock.json
 *
 * Usage:
 *   node scripts/sync-versions.js          # Update all version references
 *   node scripts/sync-versions.js --check  # Check without modifying (exit 1 if mismatch)
 */

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.join(__dirname, "..");
const ROOT_CARGO_TOML = path.join(ROOT, "Cargo.toml");
const NPM_DIR = path.join(ROOT, "npm");

const NPM_PACKAGES = [
  "onshape-mcp",
  "opencode-auth",
  "linux-x64",
  "linux-arm64",
  "darwin-x64",
  "darwin-arm64",
  "win32-x64",
];

// Internal crates whose version entries in [workspace.dependencies] must match.
const INTERNAL_CRATES = [
  "onshape-client-core",
  "onshape-client-io",
  "onshape-mcp-core",
  "onshape-mcp-io",
  "onshape-mcp-resources",
];

// ---------------------------------------------------------------------------
// Cargo version helpers
// ---------------------------------------------------------------------------

function getWorkspaceVersion() {
  const content = fs.readFileSync(ROOT_CARGO_TOML, "utf8");
  // Extract the [workspace.package] section, then find version within it.
  const section = content.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/);
  const match = section?.[1]?.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(
      `Could not find [workspace.package] version in ${ROOT_CARGO_TOML}`,
    );
  }
  return match[1];
}

/**
 * For each internal crate in [workspace.dependencies], extract the current
 * version string from its inline table.
 *
 * Returns an array of { crate, currentVersion, expected } objects for mismatches,
 * or an empty array if all match.
 */
function checkWorkspaceDepsVersions(expectedVersion) {
  const content = fs.readFileSync(ROOT_CARGO_TOML, "utf8");
  const mismatches = [];

  for (const crate of INTERNAL_CRATES) {
    // Match: crate-name = { path = "...", version = "X.Y.Z" }
    // The version field may appear before or after path.
    const pattern = new RegExp(
      `^(${escapeRegex(crate)}\\s*=\\s*\\{[^}]*)version\\s*=\\s*"([^"]+)"`,
      "m",
    );
    const match = content.match(pattern);
    if (!match) {
      mismatches.push({
        crate,
        currentVersion: "<not found>",
        expected: expectedVersion,
      });
      continue;
    }
    if (match[2] !== expectedVersion) {
      mismatches.push({
        crate,
        currentVersion: match[2],
        expected: expectedVersion,
      });
    }
  }

  return mismatches;
}

/**
 * Update the version field for each internal crate in [workspace.dependencies]
 * to match the workspace version. Returns true if any changes were made.
 */
function updateWorkspaceDepsVersions(expectedVersion) {
  let content = fs.readFileSync(ROOT_CARGO_TOML, "utf8");
  let changed = false;

  for (const crate of INTERNAL_CRATES) {
    // Replace version = "..." within the inline table for this crate.
    const pattern = new RegExp(
      `^(${escapeRegex(crate)}\\s*=\\s*\\{[^}]*version\\s*=\\s*)"([^"]+)"`,
      "m",
    );
    const match = content.match(pattern);
    if (match && match[2] !== expectedVersion) {
      content = content.replace(pattern, `$1"${expectedVersion}"`);
      changed = true;
    }
  }

  if (changed) {
    fs.writeFileSync(ROOT_CARGO_TOML, content);
  }

  return changed;
}

// ---------------------------------------------------------------------------
// Cargo.lock helpers
// ---------------------------------------------------------------------------

function checkCargoLock() {
  try {
    const [cmd, args] =
      process.platform === "win32"
        ? ["cmd.exe", ["/c", "cargo", "update", "--workspace", "--locked"]]
        : ["cargo", ["update", "--workspace", "--locked"]];
    execFileSync(cmd, args, { cwd: ROOT, stdio: "pipe" });
    return []; // no mismatches
  } catch (err) {
    const stderr = err?.stderr?.toString()?.trim();
    return [
      stderr
        ? `Cargo.lock check failed: ${stderr}`
        : "Cargo.lock is out of date (cargo update --workspace --locked failed)",
    ];
  }
}

function updateCargoLock() {
  console.log("Updating Cargo.lock...");
  try {
    const [cmd, args] =
      process.platform === "win32"
        ? ["cmd.exe", ["/c", "cargo", "update", "--workspace"]]
        : ["cargo", ["update", "--workspace"]];
    execFileSync(cmd, args, { cwd: ROOT, stdio: "pipe" });
    console.log("UPDATED: Cargo.lock");
    return true;
  } catch (err) {
    console.error("WARNING: Failed to update Cargo.lock:", err.message);
    if (err.stderr) {
      console.error("cargo output:", err.stderr.toString());
    }
    console.error("  Run 'cargo update --workspace' manually to fix.");
    return false;
  }
}

// ---------------------------------------------------------------------------
// npm helpers
// ---------------------------------------------------------------------------

function readPackageJson(pkgPath) {
  return JSON.parse(fs.readFileSync(pkgPath, "utf8"));
}

function writePackageJson(pkgPath, pkg) {
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
}

function updatePackageVersion(pkgPath, version) {
  const pkg = readPackageJson(pkgPath);
  let changed = false;

  if (pkg.version !== version) {
    pkg.version = version;
    changed = true;
  }

  // Update optionalDependencies if present (main package)
  if (pkg.optionalDependencies) {
    for (const dep of Object.keys(pkg.optionalDependencies)) {
      if (dep.startsWith("@onshape-mcp/")) {
        if (pkg.optionalDependencies[dep] !== version) {
          pkg.optionalDependencies[dep] = version;
          changed = true;
        }
      }
    }
  }

  if (changed) {
    writePackageJson(pkgPath, pkg);
  }

  return changed;
}

function checkPackageVersion(pkgPath, version) {
  const pkg = readPackageJson(pkgPath);
  const mismatches = [];

  if (pkg.version !== version) {
    mismatches.push(`version: ${pkg.version} (expected ${version})`);
  }

  if (pkg.optionalDependencies) {
    for (const [dep, depVersion] of Object.entries(pkg.optionalDependencies)) {
      if (dep.startsWith("@onshape-mcp/") && depVersion !== version) {
        mismatches.push(`${dep}: ${depVersion} (expected ${version})`);
      }
    }
  }

  return mismatches;
}

function checkLockfileVersion(lockPath, version) {
  const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
  const mismatches = [];

  if (!lock.lockfileVersion || lock.lockfileVersion < 2) {
    mismatches.push(
      `lockfileVersion: ${lock.lockfileVersion ?? "missing"} (expected >= 2)`,
    );
  }

  if (lock.version !== version) {
    mismatches.push(`version: ${lock.version} (expected ${version})`);
  }

  const rootPkg = lock.packages?.[""];
  if (!rootPkg || typeof rootPkg !== "object") {
    mismatches.push('packages[""] entry is missing');
  } else {
    if (rootPkg.version !== version) {
      mismatches.push(
        `packages[""].version: ${rootPkg.version} (expected ${version})`,
      );
    }

    if (rootPkg.optionalDependencies) {
      for (const [dep, depVersion] of Object.entries(
        rootPkg.optionalDependencies,
      )) {
        if (dep.startsWith("@onshape-mcp/") && depVersion !== version) {
          mismatches.push(
            `packages[""].optionalDependencies["${dep}"]: ${depVersion} (expected ${version})`,
          );
        }
      }
    }
  }

  return mismatches;
}

function updateNpmLockfile(pkgDir) {
  console.log("Updating npm/onshape-mcp/package-lock.json...");
  try {
    const [cmd, args] =
      process.platform === "win32"
        ? ["cmd.exe", ["/c", "npm", "install", "--package-lock-only"]]
        : ["npm", ["install", "--package-lock-only"]];
    execFileSync(cmd, args, {
      cwd: pkgDir,
      stdio: "pipe",
    });
    console.log("UPDATED: npm/onshape-mcp/package-lock.json");
    return true;
  } catch (err) {
    console.error(
      "WARNING: Failed to update package-lock.json:",
      err.message,
    );
    if (err.stderr) {
      console.error("npm output:", err.stderr.toString());
    }
    console.error(
      "  Run 'npm install' in npm/onshape-mcp/ manually to fix.",
    );
    return false;
  }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

function escapeRegex(str) {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main() {
  const checkOnly = process.argv.includes("--check");
  const version = getWorkspaceVersion();

  console.log(`Workspace version: ${version}`);
  console.log(`Mode: ${checkOnly ? "check" : "update"}`);
  console.log();

  let hasErrors = false;

  // --- Cargo workspace dependencies ---
  console.log("=== Cargo workspace dependencies ===");
  if (checkOnly) {
    const depMismatches = checkWorkspaceDepsVersions(version);
    if (depMismatches.length > 0) {
      for (const m of depMismatches) {
        console.error(
          `MISMATCH: [workspace.dependencies] ${m.crate}: ${m.currentVersion} (expected ${m.expected})`,
        );
      }
      hasErrors = true;
    } else {
      console.log("OK: [workspace.dependencies] internal crate versions");
    }
  } else {
    const changed = updateWorkspaceDepsVersions(version);
    if (changed) {
      console.log(
        "UPDATED: [workspace.dependencies] internal crate versions",
      );
    } else {
      console.log(
        "OK: [workspace.dependencies] internal crate versions (no changes)",
      );
    }
  }
  console.log();

  // --- Cargo.lock ---
  console.log("=== Cargo.lock ===");
  if (checkOnly) {
    const lockMismatches = checkCargoLock();
    if (lockMismatches.length > 0) {
      for (const m of lockMismatches) {
        console.error(`MISMATCH: ${m}`);
      }
      hasErrors = true;
    } else {
      console.log("OK: Cargo.lock");
    }
  } else {
    if (!updateCargoLock()) {
      hasErrors = true;
    }
  }
  console.log();

  // --- npm package.json files ---
  console.log("=== npm packages ===");
  for (const pkg of NPM_PACKAGES) {
    const pkgPath = path.join(NPM_DIR, pkg, "package.json");

    if (!fs.existsSync(pkgPath)) {
      console.error(`ERROR: ${pkgPath} not found`);
      hasErrors = true;
      continue;
    }

    if (checkOnly) {
      const mismatches = checkPackageVersion(pkgPath, version);
      if (mismatches.length > 0) {
        console.error(`MISMATCH: npm/${pkg}/package.json`);
        for (const m of mismatches) {
          console.error(`  - ${m}`);
        }
        hasErrors = true;
      } else {
        console.log(`OK: npm/${pkg}/package.json`);
      }
    } else {
      const changed = updatePackageVersion(pkgPath, version);
      if (changed) {
        console.log(`UPDATED: npm/${pkg}/package.json`);
      } else {
        console.log(`OK: npm/${pkg}/package.json (no changes)`);
      }
    }
  }
  console.log();

  // --- npm package-lock.json ---
  console.log("=== npm lockfile ===");
  const mainPkgDir = path.join(NPM_DIR, "onshape-mcp");
  const lockPath = path.join(mainPkgDir, "package-lock.json");

  if (checkOnly) {
    if (!fs.existsSync(lockPath)) {
      console.error("ERROR: npm/onshape-mcp/package-lock.json not found");
      hasErrors = true;
    } else {
      const lockMismatches = checkLockfileVersion(lockPath, version);
      if (lockMismatches.length > 0) {
        console.error("MISMATCH: npm/onshape-mcp/package-lock.json");
        for (const m of lockMismatches) {
          console.error(`  - ${m}`);
        }
        hasErrors = true;
      } else {
        console.log("OK: npm/onshape-mcp/package-lock.json");
      }
    }
  } else {
    if (!updateNpmLockfile(mainPkgDir)) {
      hasErrors = true;
    }
  }

  if (hasErrors) {
    console.error();
    console.error("Version mismatch detected. Run without --check to fix.");
    process.exit(1);
  }
}

main();
