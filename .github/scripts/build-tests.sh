#!/bin/sh
# Build and archive tests with nextest
# Assumes Rust and nextest are already installed

set -ex

echo "::group::Build and archive tests"
cargo nextest archive --all-features --archive-file target/nextest-archive.tar.zst
echo "::endgroup::"

echo "::group::Verify static linking"
# Note: We use 'file' instead of 'ldd' because musl's ldd is a simple wrapper
# that outputs the loader path even for static binaries, unlike glibc's ldd
# which says "not a dynamic executable" for static binaries.
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
echo "::endgroup::"
