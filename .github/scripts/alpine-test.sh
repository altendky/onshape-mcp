#!/usr/bin/env sh
# Run tests in Alpine container (musl environment)
# Usage: alpine-test.sh

set -eux

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Install basic dependencies for running tests
apk add --no-cache curl bash tar gzip

# Setup nextest
"$REPO_ROOT/.github/actions/setup-nextest/setup.sh"

# Run tests from archive with CI profile (produces JUnit XML for Mergify CI Insights)
cargo-nextest nextest run --profile ci --archive-file nextest-archive.tar.zst --workspace-remap .
