"use strict";

const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { describe, it } = require("node:test");

const {
  checkLockfileVersion,
  checkNpmPublishInventory,
  getNpmPackageDirectories,
  updateNpmLockfile,
  validateNpmLockfile,
} = require("./sync-versions.js");

function validLockfile(overrides = {}) {
  return {
    version: "0.5.0",
    lockfileVersion: 3,
    packages: { "": { version: "0.5.0" } },
    ...overrides,
  };
}

function temporaryDirectory(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "sync-versions-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

describe("npm lockfile validation", () => {
  it("requires object containers", () => {
    const cases = [
      [[], "top-level value must be a non-array object"],
      [null, "top-level value must be a non-array object"],
      [validLockfile({ packages: [] }), "packages must be a non-array object"],
      [
        validLockfile({ packages: null }),
        "packages must be a non-array object",
      ],
      [
        validLockfile({ packages: { "": [] } }),
        'packages[""] must be a non-array object',
      ],
      [
        validLockfile({ packages: { "": null } }),
        'packages[""] must be a non-array object',
      ],
    ];

    for (const [lockfile, expected] of cases) {
      assert.ok(validateNpmLockfile(lockfile).includes(expected));
    }
  });

  it("requires an integer lockfileVersion of at least 2", () => {
    assert.deepStrictEqual(validateNpmLockfile(validLockfile()), []);
    assert.deepStrictEqual(
      validateNpmLockfile(validLockfile({ lockfileVersion: 2 })),
      [],
    );

    for (const lockfileVersion of [1, 2.5, "3", null]) {
      assert.ok(
        validateNpmLockfile(validLockfile({ lockfileVersion })).some((error) =>
          error.includes("expected an integer >= 2"),
        ),
      );
    }
  });

  it("reports invalid JSON in check mode", (t) => {
    const lockPath = path.join(temporaryDirectory(t), "package-lock.json");
    fs.writeFileSync(lockPath, "{");

    assert.match(checkLockfileVersion(lockPath, "0.5.0")[0], /^invalid JSON:/);
  });

  it("does not modify malformed lockfiles in update mode", (t) => {
    const packageDirectory = temporaryDirectory(t);
    const lockPath = path.join(packageDirectory, "package-lock.json");
    const malformed = JSON.stringify(validLockfile({ packages: { "": [] } }));
    fs.writeFileSync(lockPath, malformed);

    const errors = [];
    const originalError = console.error;
    const originalLog = console.log;
    console.error = (...args) => errors.push(args.join(" "));
    console.log = () => {};
    try {
      assert.strictEqual(updateNpmLockfile(packageDirectory, "0.6.0"), false);
    } finally {
      console.error = originalError;
      console.log = originalLog;
    }

    assert.strictEqual(fs.readFileSync(lockPath, "utf8"), malformed);
    assert.ok(
      errors.some((error) => error.includes("must be a non-array object")),
    );
  });
});

describe("npm package discovery", () => {
  it("discovers npm packages dynamically without including the worker", (t) => {
    const root = temporaryDirectory(t);
    const npmDirectory = path.join(root, "npm");
    for (const packageName of ["zeta", "alpha"]) {
      const packageDirectory = path.join(npmDirectory, packageName);
      fs.mkdirSync(packageDirectory, { recursive: true });
      fs.writeFileSync(path.join(packageDirectory, "package.json"), "{}\n");
    }
    fs.mkdirSync(path.join(npmDirectory, "not-a-package"));

    const workerDirectory = path.join(root, "workers", "oauth-proxy");
    fs.mkdirSync(workerDirectory, { recursive: true });
    fs.writeFileSync(path.join(workerDirectory, "package.json"), "{}\n");

    assert.deepStrictEqual(getNpmPackageDirectories(npmDirectory), [
      "alpha",
      "zeta",
    ]);
    assert.deepStrictEqual(
      checkNpmPublishInventory(["alpha", "zeta"], npmDirectory),
      [],
    );
    assert.deepStrictEqual(
      checkNpmPublishInventory(["alpha", "missing"], npmDirectory),
      [
        "npm/zeta is not in the publish inventory",
        "publish inventory package npm/missing does not exist",
      ],
    );
  });
});
