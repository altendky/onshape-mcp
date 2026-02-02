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
x86_64)
	# Use prebuilt musl binary for x86_64
	curl -LsSf "https://get.nexte.st/latest/linux-musl" | tar zxf - -C /usr/local/bin
	;;
aarch64)
	# No prebuilt musl binary for ARM, and glibc binary doesn't work with gcompat
	# (__res_init symbol missing). Build from source instead.
	cargo install cargo-nextest --locked
	;;
*)
	echo "Unsupported architecture: $ARCH" && exit 1
	;;
esac

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
