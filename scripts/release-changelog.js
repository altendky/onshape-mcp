#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const CHANGELOG_PATH = path.join(
  ROOT,
  "docs",
  "src",
  "project",
  "changelog.md",
);
const SEMVER_PATTERN = /^\d+\.\d+\.\d+$/;
const DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

function validateDate(date) {
  if (!DATE_PATTERN.test(date)) {
    return false;
  }
  const parsed = new Date(`${date}T00:00:00Z`);
  return (
    !Number.isNaN(parsed.valueOf()) &&
    parsed.toISOString().slice(0, 10) === date
  );
}

function currentLocalDate() {
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function parseChangelog(content) {
  const headingPattern = /^## \[([^\]\r\n]+)\](?: - ([^\r\n]+))?\r?$/gm;
  const candidateHeadings = content.match(/^## \[.*$/gm) ?? [];
  const matches = [...content.matchAll(headingPattern)];
  if (matches.length !== candidateHeadings.length) {
    throw new Error("Malformed changelog release heading");
  }
  if (matches.length === 0) {
    throw new Error("No changelog release headings found");
  }

  const seen = new Set();
  const sections = matches.map((match, index) => {
    const name = match[1];
    const date = match[2];
    if (seen.has(name)) {
      throw new Error(`Duplicate changelog section: ${name}`);
    }
    seen.add(name);

    if (name === "Unreleased") {
      if (date !== undefined) {
        throw new Error("Unreleased changelog section must not have a date");
      }
    } else {
      if (!SEMVER_PATTERN.test(name)) {
        throw new Error(`Invalid changelog version heading: ${name}`);
      }
      if (date === undefined || !validateDate(date)) {
        throw new Error(
          `Invalid or missing date for changelog version ${name}`,
        );
      }
    }

    const start = match.index;
    const headingEnd = start + match[0].length;
    const end = matches[index + 1]?.index ?? content.length;
    return {
      name,
      date,
      heading: match[0],
      start,
      end,
      body: content.slice(headingEnd, end).trim(),
    };
  });

  if (sections[0].name !== "Unreleased") {
    throw new Error("Unreleased must be the first changelog release section");
  }
  return sections;
}

function finalizeChangelog(content, version, date) {
  if (!SEMVER_PATTERN.test(version)) {
    throw new Error(`Invalid release version: ${version}`);
  }
  if (!validateDate(date)) {
    throw new Error(`Invalid release date: ${date}`);
  }

  const sections = parseChangelog(content);
  const unreleased = sections[0];
  if (sections.some((section) => section.name === version)) {
    throw new Error(`Changelog section already exists for ${version}`);
  }
  if (unreleased.body.length === 0) {
    throw new Error("Unreleased changelog section is empty");
  }

  const prefix = content.slice(0, unreleased.start);
  const suffix = content.slice(unreleased.end).trimStart();
  let finalized = `${prefix}## [Unreleased]\n\n## [${version}] - ${date}\n\n${unreleased.body}\n`;
  if (suffix.length > 0) {
    finalized += `\n${suffix}`;
  }
  if (!finalized.endsWith("\n")) {
    finalized += "\n";
  }
  return finalized;
}

function extractReleaseSection(content, version, required = true) {
  if (!SEMVER_PATTERN.test(version)) {
    throw new Error(`Invalid release version: ${version}`);
  }
  const section = parseChangelog(content).find(
    (candidate) => candidate.name === version,
  );
  if (!section) {
    if (!required) {
      return null;
    }
    throw new Error(`Missing changelog section for ${version}`);
  }
  if (section.body.length === 0) {
    throw new Error(`Changelog section for ${version} is empty`);
  }
  return `${section.heading}\n\n${section.body}\n`;
}

function main() {
  const [command, version, ...args] = process.argv.slice(2);
  if (!command || !version) {
    throw new Error(
      "Usage: release-changelog.js <finalize|extract|validate-if-present> <version> [--date YYYY-MM-DD]",
    );
  }

  const content = fs.readFileSync(CHANGELOG_PATH, "utf8");
  if (command === "finalize") {
    const dateIndex = args.indexOf("--date");
    const date =
      dateIndex === -1 ? currentLocalDate() : args[dateIndex + 1];
    if (!date) {
      throw new Error("--date requires a YYYY-MM-DD value");
    }
    fs.writeFileSync(CHANGELOG_PATH, finalizeChangelog(content, version, date));
    console.log(`Finalized changelog for ${version} (${date})`);
    return;
  }
  if (command === "extract") {
    process.stdout.write(extractReleaseSection(content, version));
    return;
  }
  if (command === "validate-if-present") {
    const section = extractReleaseSection(content, version, false);
    console.log(
      section
        ? `Validated changelog section for ${version}`
        : `No finalized changelog section for ${version}`,
    );
    return;
  }
  throw new Error(`Unknown command: ${command}`);
}

if (require.main === module) {
  try {
    main();
  } catch (err) {
    console.error(`ERROR: ${err.message}`);
    process.exit(1);
  }
}

module.exports = {
  extractReleaseSection,
  finalizeChangelog,
  parseChangelog,
};
