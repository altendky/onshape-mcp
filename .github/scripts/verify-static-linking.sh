#!/usr/bin/env sh
# Verify that binaries are statically linked
#
# Runs on Ubuntu build runners after building with musl target.
# Uses 'file' command which correctly identifies static vs dynamic linking.
#
# Note: We use 'file' instead of 'ldd' because musl's ldd is a simple wrapper
# that outputs the loader path even for static binaries, unlike glibc's ldd
# which says "not a dynamic executable" for static binaries.

set -eux

METADATA=${1:?Usage: verify-static-linking.sh NEXTEST_BINARIES_JSON}

# Install file command if not present
if ! command -v file >/dev/null 2>&1; then
	if command -v apt-get >/dev/null 2>&1; then
		sudo apt-get update && sudo apt-get install -y file
	else
		echo "ERROR: 'file' not available and no supported package manager found" >&2
		exit 1
	fi
fi

# Nextest records actual executable paths, including Cargo's build-dir layout.
python - "$METADATA" <<'PY'
import json
import os
from pathlib import Path
import subprocess
import sys


def fail(message):
    print(f"ERROR: {message}", file=sys.stderr)
    raise SystemExit(1)


try:
    metadata = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    build = metadata["rust-build-meta"]
    target_directory = Path(build["target-directory"])
    binaries = {
        Path(binary["binary-path"])
        for binary in metadata["rust-binaries"].values()
        if binary["build-platform"] == "target"
    }
    for package_binaries in build["non-test-binaries"].values():
        binaries.update(
            target_directory / binary["path"]
            for binary in package_binaries
            if binary["kind"] == "bin-exe" and binary["build-platform"] == "target"
        )
except (AttributeError, KeyError, OSError, TypeError, ValueError) as error:
    fail(f"cannot read Nextest binary metadata: {error}")

if not binaries:
    fail("Nextest metadata contains no target executables")

for binary in sorted(binaries):
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail(f"missing or non-executable target binary: {binary}")
    try:
        description = subprocess.run(
            ["file", "--brief", "--", str(binary)],
            check=True,
            capture_output=True,
            text=True,
            env={**os.environ, "LC_ALL": "C"},
        ).stdout.strip()
    except subprocess.CalledProcessError as error:
        fail(f"cannot inspect {binary}: {error.stderr.strip()}")
    if not description.startswith("ELF ") or "executable" not in description:
        fail(f"target binary is not an ELF executable: {binary}: {description}")
    if "statically linked" not in description and "static-pie linked" not in description:
        fail(f"target binary is dynamically linked: {binary}: {description}")
    print(f"OK: {binary} is statically linked")

print(f"Checked {len(binaries)} target executables")
PY
