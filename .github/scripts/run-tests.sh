#!/bin/sh
# Run tests from nextest archive
# Assumes nextest is already installed

set -ex

echo "::group::Run tests"
cargo-nextest nextest run --archive-file nextest-archive.tar.zst --workspace-remap .
echo "::endgroup::"
