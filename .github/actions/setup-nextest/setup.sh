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
	# No prebuilt musl binary for ARM, and glibc binary doesn't work with gcompat
	# (__res_init symbol missing). Build from source instead.

	# Install build dependencies if not present
	if ! command -v gcc >/dev/null 2>&1; then
		apk add --no-cache gcc musl-dev
	fi

	# Install Rust if not present (minimal profile for faster install)
	if ! command -v cargo >/dev/null 2>&1; then
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
		. "$HOME/.cargo/env"
	fi

	cargo install cargo-nextest --locked
	;;
*)
	echo "Unsupported architecture: $ARCH" && exit 1
	;;
esac
