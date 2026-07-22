"use strict";

const assert = require("node:assert");
const { describe, it } = require("node:test");

const {
  extractReleaseSection,
  finalizeChangelog,
  parseChangelog,
} = require("./release-changelog.js");

const PREAMBLE = "# Changelog\n\nProject changes.\n\n";

describe("release changelog", () => {
  it("moves Unreleased content into a dated release section", () => {
    const input = `${PREAMBLE}## [Unreleased]\n\n### Added\n\n- New feature\n`;
    const output = finalizeChangelog(input, "0.5.0", "2026-07-21");

    assert.match(output, /## \[Unreleased\]\n\n## \[0\.5\.0\] - 2026-07-21/);
    assert.strictEqual(parseChangelog(output)[0].body, "");
    assert.strictEqual(
      extractReleaseSection(output, "0.5.0"),
      "## [0.5.0] - 2026-07-21\n\n### Added\n\n- New feature\n",
    );
  });

  it("preserves older release sections", () => {
    const input = `${PREAMBLE}## [Unreleased]\n\n- New\n\n## [0.4.0] - 2026-06-01\n\n- Old\n`;
    const output = finalizeChangelog(input, "0.5.0", "2026-07-21");

    assert.match(output, /## \[0\.4\.0\] - 2026-06-01\n\n- Old/);
  });

  it("rejects empty, duplicate, and pre-finalized state", () => {
    assert.throws(
      () =>
        finalizeChangelog(
          `${PREAMBLE}## [Unreleased]\n`,
          "0.5.0",
          "2026-07-21",
        ),
      /Unreleased changelog section is empty/,
    );
    assert.throws(
      () =>
        finalizeChangelog(
          `${PREAMBLE}## [Unreleased]\n\n- A\n\n## [Unreleased]\n\n- B\n`,
          "0.5.0",
          "2026-07-21",
        ),
      /Duplicate changelog section: Unreleased/,
    );
    assert.throws(
      () =>
        finalizeChangelog(
          `${PREAMBLE}## [Unreleased]\n\n- A\n\n## [0.5.0] - 2026-07-20\n\n- Existing\n`,
          "0.5.0",
          "2026-07-21",
        ),
      /Changelog section already exists for 0.5.0/,
    );
  });

  it("rejects malformed headings and dates", () => {
    assert.throws(
      () =>
        parseChangelog(
          `${PREAMBLE}## [Unreleased]\n\n- A\n\n## [0.4.0] soon\n`,
        ),
      /Malformed changelog release heading/,
    );
    assert.throws(
      () =>
        finalizeChangelog(
          `${PREAMBLE}## [Unreleased]\n\n- A\n`,
          "0.5.0",
          "2026-02-30",
        ),
      /Invalid release date/,
    );
  });

  it("allows validation when the target section is not finalized yet", () => {
    const input = `${PREAMBLE}## [Unreleased]\n\n- Pending\n`;
    assert.strictEqual(extractReleaseSection(input, "0.5.0", false), null);
    assert.throws(
      () => extractReleaseSection(input, "0.5.0"),
      /Missing changelog section/,
    );
  });
});
