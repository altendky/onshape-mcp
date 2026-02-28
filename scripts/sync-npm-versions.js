#!/usr/bin/env node
"use strict";

/**
 * Syncs npm package versions from Cargo.toml.
 *
 * Updates all package.json files and regenerates package-lock.json
 * to keep versions in sync with the Cargo workspace version.
 *
 * Usage:
 *   node scripts/sync-npm-versions.js          # Update all package.json and package-lock.json files
 *   node scripts/sync-npm-versions.js --check  # Check without modifying (exit 1 if mismatch)
 */

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.join(__dirname, "..");
const CARGO_TOML = path.join(ROOT, "crates", "onshape-mcp", "Cargo.toml");
const NPM_DIR = path.join(ROOT, "npm");

const PACKAGES = [
  "onshape-mcp",
  "opencode-auth",
  "linux-x64",
  "linux-arm64",
  "darwin-x64",
  "darwin-arm64",
  "win32-x64",
];

function getCargoVersion() {
  const content = fs.readFileSync(CARGO_TOML, "utf8");
  const match = content.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`Could not find version in ${CARGO_TOML}`);
  }
  return match[1];
}

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

function updateLockfile(pkgDir) {
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

function main() {
  const checkOnly = process.argv.includes("--check");
  const version = getCargoVersion();

  console.log(`Cargo version: ${version}`);
  console.log(`Mode: ${checkOnly ? "check" : "update"}`);
  console.log();

  let hasErrors = false;

  for (const pkg of PACKAGES) {
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

  // Handle package-lock.json for the main package
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
    if (!updateLockfile(mainPkgDir)) {
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
