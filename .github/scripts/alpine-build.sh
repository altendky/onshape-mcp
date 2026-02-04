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

# Override rust-toolchain.toml to use the requested version
# This is required because proc-macro support on musl requires newer toolchains
# than the MSRV specified in rust-toolchain.toml
export RUSTUP_TOOLCHAIN="$RUST_VERSION"

# Setup nextest
"$REPO_ROOT/.github/actions/setup-nextest/setup.sh"

# Build and archive tests
cargo nextest archive --all-features --archive-file target/nextest-archive.tar.zst

# Verify static linking
"$SCRIPT_DIR/verify-static-linking.sh"
