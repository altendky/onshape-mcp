#!/bin/sh
# Run tests in Alpine container (musl environment)
# Usage: alpine-test.sh

set -ex

# Install dependencies
apk add --no-cache curl bash tar gzip

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
	# Pin to 0.9.114 for MSRV 1.88 compatibility (0.9.115+ requires 1.89)
	apk add --no-cache gcc musl-dev
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
	. "$HOME/.cargo/env"
	cargo install cargo-nextest --locked --version 0.9.114
	;;
*)
	echo "Unsupported architecture: $ARCH" && exit 1
	;;
esac

# Run tests from archive
cargo-nextest nextest run --archive-file nextest-archive.tar.zst --workspace-remap .
