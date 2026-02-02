#!/bin/sh
# Build in Alpine container for musl static linking
# Usage: alpine-build.sh <rust-version>

set -ex

RUST_VERSION="${1:?Usage: alpine-build.sh <rust-version>}"

# Install dependencies
apk add --no-cache curl bash gcc musl-dev file

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain "$RUST_VERSION"
. "$HOME/.cargo/env"

# Install nextest (architecture-aware)
ARCH=$(uname -m)
case "$ARCH" in
x86_64) NEXTEST_URL="https://get.nexte.st/latest/linux-musl" ;;
aarch64) NEXTEST_URL="https://get.nexte.st/latest/linux-arm-musl" ;;
*) echo "Unsupported architecture: $ARCH" && exit 1 ;;
esac
curl -LsSf "$NEXTEST_URL" | tar zxf - -C /usr/local/bin

# Build and archive tests (static linking configured in .cargo/config.toml)
cargo nextest archive --all-features --archive-file target/nextest-archive.tar.zst

# Verify static linking
echo 'Checking for statically linked binaries...'
for bin in target/debug/deps/*; do
	if [ -f "$bin" ] && [ -x "$bin" ]; then
		if file "$bin" | grep -q 'ELF.*executable'; then
			if ldd "$bin" 2>&1 | grep -qE 'not a dynamic executable|statically linked'; then
				echo "OK: $bin is statically linked"
			else
				echo "ERROR: $bin is dynamically linked"
				ldd "$bin" 2>&1 || true
				exit 1
			fi
		fi
	fi
done
