#!/bin/sh
# Run tests in Alpine container (musl environment)
# Usage: alpine-test.sh

set -ex

SCRIPT_DIR="$(dirname "$0")"

# Install dependencies
apk add --no-cache curl bash tar gzip

# Install nextest (handles Rust installation for ARM if needed)
"$SCRIPT_DIR/install-nextest-musl.sh"

# Source cargo env if it was installed (ARM case)
if [ -f "$HOME/.cargo/env" ]; then
	. "$HOME/.cargo/env"
fi

# Run tests from archive
cargo-nextest nextest run --archive-file nextest-archive.tar.zst --workspace-remap .
