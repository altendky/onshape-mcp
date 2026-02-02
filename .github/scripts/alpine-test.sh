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
	NEXTEST_URL="https://get.nexte.st/latest/linux-musl"
	;;
aarch64)
	# No musl binary available for ARM; use glibc binary with compat layer
	apk add --no-cache gcompat
	NEXTEST_URL="https://get.nexte.st/latest/linux-arm"
	;;
*)
	echo "Unsupported architecture: $ARCH" && exit 1
	;;
esac
curl -LsSf "$NEXTEST_URL" | tar zxf - -C /usr/local/bin

# Run tests from archive
cargo-nextest nextest run --archive-file nextest-archive.tar.zst --workspace-remap .
