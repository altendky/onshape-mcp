#!/bin/sh
# Run tests in Alpine container (musl environment)
# Usage: alpine-test.sh
#
# This script orchestrates the test process by calling separate scripts
# for each phase: install dependencies, install nextest, and run tests.

set -ex

SCRIPT_DIR="$(dirname "$0")"

echo "::group::Install Alpine dependencies"
apk add --no-cache curl bash tar gzip
echo "::endgroup::"

# Install nextest (handles Rust installation for ARM if needed)
"$SCRIPT_DIR/install-nextest-musl.sh"

# Source cargo env if it was installed (ARM case)
if [ -f "$HOME/.cargo/env" ]; then
	. "$HOME/.cargo/env"
fi

# Run tests from archive
"$SCRIPT_DIR/run-tests.sh"
