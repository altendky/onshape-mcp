#!/bin/sh
# Build in Alpine container for musl static linking
# Usage: alpine-build.sh <rust-version>

set -ex

RUST_VERSION="${1:?Usage: alpine-build.sh <rust-version>}"
SCRIPT_DIR="$(dirname "$0")"

# Install dependencies
apk add --no-cache curl bash gcc musl-dev file

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain "$RUST_VERSION"
. "$HOME/.cargo/env"

# Install nextest
"$SCRIPT_DIR/install-nextest-musl.sh"

# Build and archive tests (static linking configured in .cargo/config.toml)
cargo nextest archive --all-features --archive-file target/nextest-archive.tar.zst

# Verify static linking
# Note: We use 'file' instead of 'ldd' because musl's ldd is a simple wrapper
# that outputs the loader path even for static binaries, unlike glibc's ldd
# which says "not a dynamic executable" for static binaries.
echo 'Checking for statically linked binaries...'
for bin in target/debug/deps/*; do
	if [ -f "$bin" ] && [ -x "$bin" ]; then
		if file "$bin" | grep -q 'ELF.*executable'; then
			if file "$bin" | grep -qE 'statically linked|static-pie linked'; then
				echo "OK: $bin is statically linked"
			else
				echo "ERROR: $bin is dynamically linked"
				file "$bin"
				exit 1
			fi
		fi
	fi
done
