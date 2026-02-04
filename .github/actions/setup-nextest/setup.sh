#!/usr/bin/env sh
# Install cargo-nextest in Alpine container
# Usage: setup.sh
#
# Handles architecture-specific installation:
#   x86_64:  Downloads prebuilt musl binary
#   aarch64: Builds from source (installs Rust if needed)

set -eux

ARCH=$(uname -m)
case "$ARCH" in
x86_64)
	# Use prebuilt musl binary for x86_64
	curl -LsSf "https://get.nexte.st/latest/linux-musl" | tar zxf - -C /usr/local/bin
	;;
aarch64)
	# Use prebuilt musl binary for aarch64 (available since nextest 0.9.125)
	curl -LsSf "https://get.nexte.st/latest/linux-arm-musl" | tar zxf - -C /usr/local/bin
	;;
*)
	echo "Unsupported architecture: $ARCH" && exit 1
	;;
esac
