#!/usr/bin/env sh
# Build in Alpine container for musl static linking
# Usage: alpine-build.sh <rust-version>

set -eux

RUST_VERSION="${1:?Usage: alpine-build.sh <rust-version>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Setup Rust
"$REPO_ROOT/.github/actions/setup-rust/setup.sh" "$RUST_VERSION"
# shellcheck source=/dev/null
. "$HOME/.cargo/env"

# Setup nextest
"$REPO_ROOT/.github/actions/setup-nextest/setup.sh"

# Build and archive tests
cargo nextest archive --all-features --archive-file target/nextest-archive.tar.zst

# Verify static linking
"$SCRIPT_DIR/verify-static-linking.sh"
