#!/usr/bin/env node
"use strict";

// Mock binary for testing bin.js wrapper
// Outputs JSON to stdout with information about how it was invoked
// Supports --exit N to control exit code

const args = process.argv.slice(2);

// Find --exit flag for controlling exit code
let exitCode = 0;
const exitIdx = args.indexOf("--exit");
if (exitIdx !== -1 && args[exitIdx + 1] !== undefined) {
  exitCode = parseInt(args[exitIdx + 1], 10);
  if (isNaN(exitCode)) {
    exitCode = 1;
  }
}

// Output all relevant information as JSON
const result = {
  args: args,
  cwd: process.cwd(),
  pid: process.pid,
  platform: process.platform,
  arch: process.arch,
  nodeVersion: process.version,
  env: {
    // Include select env vars that might be relevant for testing
    ONSHAPE_MCP_NPM_COMMAND: process.env.ONSHAPE_MCP_NPM_COMMAND,
  },
};

console.log(JSON.stringify(result));
process.exit(exitCode);
