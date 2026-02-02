#!/usr/bin/env sh
# Install Rust toolchain in Alpine container
# Usage: setup.sh <toolchain> [components]
#
# Arguments:
#   toolchain   - Rust toolchain version (e.g., "1.88", "stable", "beta")
#   components  - Optional comma-separated list of components (e.g., "rustfmt,clippy")

set -eux

TOOLCHAIN="${1:?Usage: setup.sh <toolchain> [components]}"
COMPONENTS="${2:-}"

# Install Alpine build dependencies
apk add --no-cache curl bash gcc musl-dev

# Install Rust via rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain "$TOOLCHAIN"

# Source cargo env for this script
. "$HOME/.cargo/env"

# Install components if specified
if [ -n "$COMPONENTS" ]; then
	# Convert comma-separated list to space-separated for rustup
	COMPONENTS_LIST=$(echo "$COMPONENTS" | tr ',' ' ' | tr -s ' ')
	rustup component add $COMPONENTS_LIST
fi
