#!/bin/sh
# Build in Alpine container for musl static linking
# Usage: alpine-build.sh <rust-version>
#
# This script orchestrates the build process by calling separate scripts
# for each phase: setup, install nextest, and build tests.

set -ex

RUST_VERSION="${1:?Usage: alpine-build.sh <rust-version>}"
SCRIPT_DIR="$(dirname "$0")"

# Phase 1: Setup Alpine environment and install Rust
"$SCRIPT_DIR/alpine-setup.sh" "$RUST_VERSION"
. "$HOME/.cargo/env"

# Phase 2: Install nextest
"$SCRIPT_DIR/install-nextest-musl.sh"

# Phase 3: Build and archive tests
"$SCRIPT_DIR/build-tests.sh"
