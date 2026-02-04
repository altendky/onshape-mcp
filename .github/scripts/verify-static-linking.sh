#!/usr/bin/env sh
# Verify that binaries are statically linked
#
# Works on both Ubuntu (with cargo-zigbuild) and Alpine environments.
# Uses 'file' command which correctly identifies static vs dynamic linking
# on both glibc and musl systems.
#
# Note: We use 'file' instead of 'ldd' because musl's ldd is a simple wrapper
# that outputs the loader path even for static binaries, unlike glibc's ldd
# which says "not a dynamic executable" for static binaries.

set -eux

# Install file command if not present
if ! command -v file >/dev/null 2>&1; then
	if command -v apk >/dev/null 2>&1; then
		apk add --no-cache file
	elif command -v apt-get >/dev/null 2>&1; then
		sudo apt-get update && sudo apt-get install -y file
	fi
fi

# Find the target directory - check for musl target dirs first, then debug
if [ -d "target/x86_64-unknown-linux-musl/debug/deps" ]; then
	DEPS_DIR="target/x86_64-unknown-linux-musl/debug/deps"
elif [ -d "target/aarch64-unknown-linux-musl/debug/deps" ]; then
	DEPS_DIR="target/aarch64-unknown-linux-musl/debug/deps"
elif [ -d "target/debug/deps" ]; then
	DEPS_DIR="target/debug/deps"
else
	echo "ERROR: No deps directory found"
	exit 1
fi

echo "Checking binaries in: $DEPS_DIR"

for bin in "$DEPS_DIR"/*; do
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
