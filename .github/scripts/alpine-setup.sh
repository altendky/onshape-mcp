#!/bin/sh
# Setup Alpine environment for Rust builds
# Usage: alpine-setup.sh <rust-version>

set -ex

RUST_VERSION="${1:?Usage: alpine-setup.sh <rust-version>}"

echo "::group::Install Alpine dependencies"
apk add --no-cache curl bash gcc musl-dev file
echo "::endgroup::"

echo "::group::Install Rust $RUST_VERSION"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain "$RUST_VERSION"
echo "::endgroup::"
