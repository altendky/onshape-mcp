#!/usr/bin/env node
"use strict";

/**
 * Syncs all project versions from the workspace version in root Cargo.toml.
 *
 * Propagates [workspace.package].version to:
 *   - [workspace.dependencies] internal crate version entries (root Cargo.toml)
 *   - Cargo.lock (via `cargo update --workspace`)
 *   - All npm package.json version fields and package-lock.json
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
 * Find every path-based dependency in [workspace.dependencies].
 *
 * The workspace uses inline tables for local dependencies so they remain
 * publishable with a registry version fallback.
 */
function getPathWorkspaceDependencies(content) {
  const section = content.match(
    /\[workspace\.dependencies\]([\s\S]*?)(?=\n\[|$)/,
  );
  if (!section) {
    throw new Error(
      `Could not find [workspace.dependencies] in ${ROOT_CARGO_TOML}`,
    );
  }

  const dependencies = [];
  const entryPattern = /^\s*([A-Za-z0-9_-]+)\s*=\s*\{([^}]*)\}/gm;
  let match;
  while ((match = entryPattern.exec(section[1])) !== null) {
    if (!/(?:^|,)\s*path\s*=\s*"[^"]+"/.test(match[2])) {
      continue;
    }
    const version = match[2].match(/(?:^|,)\s*version\s*=\s*"([^"]+)"/);
    dependencies.push({
      crate: match[1],
      currentVersion: version?.[1] ?? "<not found>",
    });
  }

  if (dependencies.length === 0) {
    throw new Error(
      `No path-based dependencies found in [workspace.dependencies] in ${ROOT_CARGO_TOML}`,
    );
  }
  return dependencies;
}

/**
 * For each path-based dependency in [workspace.dependencies], extract the
 * current version string from its inline table.
 *
 * Returns an array of { crate, currentVersion, expected } objects for mismatches,
 * or an empty array if all match.
 */
function checkWorkspaceDepsVersions(expectedVersion) {
  const content = fs.readFileSync(ROOT_CARGO_TOML, "utf8");
  const mismatches = [];

  for (const { crate, currentVersion } of getPathWorkspaceDependencies(
    content,
  )) {
    if (currentVersion !== expectedVersion) {
      mismatches.push({
        crate,
        currentVersion,
        expected: expectedVersion,
      });
    }
  }

  return mismatches;
}

/**
 * Update the version field for each path-based dependency in
 * [workspace.dependencies] to match the workspace version.
 */
function updateWorkspaceDepsVersions(expectedVersion) {
  let content = fs.readFileSync(ROOT_CARGO_TOML, "utf8");
  let changed = false;
  const dependencies = getPathWorkspaceDependencies(content);
  const sectionPattern = /\[workspace\.dependencies\]([\s\S]*?)(?=\n\[|$)/;
  let section = content.match(sectionPattern)[0];

  for (const { crate, currentVersion } of dependencies) {
    const pattern = new RegExp(
      `^(\\s*${escapeRegex(crate)}\\s*=\\s*\\{)([^}]*)(\\})`,
      "m",
    );
    if (currentVersion !== expectedVersion) {
      section = section.replace(pattern, (_entry, start, fields, end) => {
        if (/(?:^|,)\s*version\s*=\s*"[^"]+"/.test(fields)) {
          fields = fields.replace(
            /((?:^|,)\s*version\s*=\s*)"[^"]+"/,
            `$1"${expectedVersion}"`,
          );
        } else {
          const trailingWhitespace = fields.match(/\s*$/)?.[0] ?? "";
          const trimmedFields = fields.slice(
            0,
            fields.length - trailingWhitespace.length,
          );
          const separator = trimmedFields.endsWith(",") ? "" : ",";
          fields = `${trimmedFields}${separator} version = "${expectedVersion}"${trailingWhitespace}`;
        }
        return `${start}${fields}${end}`;
      });
      changed = true;
    }
  }

  if (changed) {
    content = content.replace(sectionPattern, section);
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

function getNpmPackageDirectories(npmDir = NPM_DIR) {
  const packageDirectories = fs
    .readdirSync(npmDir, { withFileTypes: true })
    .filter(
      (entry) =>
        entry.isDirectory() &&
        fs.existsSync(path.join(npmDir, entry.name, "package.json")),
    )
    .map((entry) => entry.name)
    .sort();

  if (packageDirectories.length === 0) {
    throw new Error(`No npm package.json files found under ${npmDir}`);
  }
  return packageDirectories;
}

function checkNpmPublishInventory(publishedPackages, npmDir = NPM_DIR) {
  const discovered = new Set(getNpmPackageDirectories(npmDir));
  const published = new Set(publishedPackages);
  const mismatches = [];

  for (const packageName of [...discovered].sort()) {
    if (!published.has(packageName)) {
      mismatches.push(`npm/${packageName} is not in the publish inventory`);
    }
  }
  for (const packageName of [...published].sort()) {
    if (!discovered.has(packageName)) {
      mismatches.push(
        `publish inventory package npm/${packageName} does not exist`,
      );
    }
  }
  return mismatches;
}

function updatePackageVersion(pkgPath, version) {
  const pkg = readPackageJson(pkgPath);
  let changed = false;

  if (pkg.version !== version) {
    pkg.version = version;
    changed = true;
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

  return mismatches;
}

function isNonArrayObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function validateNpmLockfile(lock) {
  if (!isNonArrayObject(lock)) {
    return ["top-level value must be a non-array object"];
  }

  const errors = [];
  if (!Number.isInteger(lock.lockfileVersion) || lock.lockfileVersion < 2) {
    errors.push(
      `lockfileVersion: ${lock.lockfileVersion ?? "missing"} (expected an integer >= 2)`,
    );
  }
  if (!isNonArrayObject(lock.packages)) {
    errors.push("packages must be a non-array object");
  } else if (!isNonArrayObject(lock.packages[""])) {
    errors.push('packages[""] must be a non-array object');
  }
  return errors;
}

function readNpmLockfile(lockPath) {
  try {
    return { lock: JSON.parse(fs.readFileSync(lockPath, "utf8")), errors: [] };
  } catch (err) {
    return { lock: null, errors: [`invalid JSON: ${err.message}`] };
  }
}

function checkLockfileVersion(lockPath, version) {
  const { lock, errors } = readNpmLockfile(lockPath);
  const mismatches = [...errors];
  if (errors.length > 0) {
    return mismatches;
  }

  mismatches.push(...validateNpmLockfile(lock));
  if (mismatches.length > 0) {
    return mismatches;
  }

  if (lock.version !== version) {
    mismatches.push(`version: ${lock.version} (expected ${version})`);
  }

  const rootPkg = lock.packages[""];
  if (rootPkg.version !== version) {
    mismatches.push(
      `packages[""].version: ${rootPkg.version} (expected ${version})`,
    );
  }

  return mismatches;
}

function updateNpmLockfile(pkgDir, version) {
  const lockPath = path.join(pkgDir, "package-lock.json");
  const lockRelPath = path.relative(ROOT, lockPath);
  console.log(`Updating ${lockRelPath}...`);

  const { lock, errors: readErrors } = readNpmLockfile(lockPath);
  const errors = [...readErrors];
  if (readErrors.length === 0) {
    errors.push(...validateNpmLockfile(lock));
  }
  if (errors.length > 0) {
    for (const error of errors) {
      console.error(`ERROR: ${lockRelPath}: ${error}`);
    }
    return false;
  }

  const rootPkg = lock.packages[""];
  let changed = false;

  if (lock.version !== version) {
    lock.version = version;
    changed = true;
  }

  if (rootPkg.version !== version) {
    rootPkg.version = version;
    changed = true;
  }

  if (changed) {
    fs.writeFileSync(lockPath, JSON.stringify(lock, null, 2) + "\n");
    console.log(`UPDATED: ${lockRelPath}`);
  } else {
    console.log(`OK: ${lockRelPath} (no changes)`);
  }

  return true;
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
      console.log("UPDATED: [workspace.dependencies] internal crate versions");
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
  const npmPackages = getNpmPackageDirectories();
  for (const pkg of npmPackages) {
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

  // --- npm package-lock.json files ---
  console.log("=== npm lockfiles ===");
  for (const pkg of npmPackages) {
    const pkgDir = path.join(NPM_DIR, pkg);
    const lockPath = path.join(pkgDir, "package-lock.json");
    if (!fs.existsSync(lockPath)) {
      continue;
    }

    if (checkOnly) {
      const lockMismatches = checkLockfileVersion(lockPath, version);
      if (lockMismatches.length > 0) {
        console.error(`MISMATCH: npm/${pkg}/package-lock.json`);
        for (const m of lockMismatches) {
          console.error(`  - ${m}`);
        }
        hasErrors = true;
      } else {
        console.log(`OK: npm/${pkg}/package-lock.json`);
      }
    } else if (!updateNpmLockfile(pkgDir, version)) {
      hasErrors = true;
    }
  }

  const mainLockPath = path.join(NPM_DIR, "onshape-mcp", "package-lock.json");
  if (!fs.existsSync(mainLockPath)) {
    console.error("ERROR: npm/onshape-mcp/package-lock.json not found");
    hasErrors = true;
  }

  if (hasErrors) {
    console.error();
    if (checkOnly) {
      console.error("Version mismatch detected. Run without --check to fix.");
    } else {
      console.error(
        "Version sync failed. See errors above and re-run after fixes.",
      );
    }
    process.exit(1);
  }
}

if (require.main === module) {
  const inventoryFlag = "--check-npm-publish-inventory";
  const inventoryIndex = process.argv.indexOf(inventoryFlag);
  if (inventoryIndex !== -1) {
    const publishedPackages = process.argv.slice(inventoryIndex + 1);
    if (publishedPackages.length === 0) {
      console.error(`ERROR: ${inventoryFlag} requires package names`);
      process.exit(1);
    }
    const mismatches = checkNpmPublishInventory(publishedPackages);
    if (mismatches.length > 0) {
      for (const mismatch of mismatches) {
        console.error(`ERROR: ${mismatch}`);
      }
      process.exit(1);
    }
    console.log("OK: npm publish inventory matches discovered packages");
  } else {
    main();
  }
}

module.exports = {
  checkLockfileVersion,
  checkNpmPublishInventory,
  getNpmPackageDirectories,
  updateNpmLockfile,
  validateNpmLockfile,
};
