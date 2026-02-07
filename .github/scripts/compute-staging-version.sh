#!/usr/bin/env bash
set -euo pipefail

# Computes a staging version for npm pre-release publishes.
#
# Usage:
#   compute-staging-version.sh <cargo-toml-path> <git-ref> <commit-sha> <run-id>
#
# Output (on stdout):
#   {version}-staging-{sanitized_ref}-{short_sha}-{run_id}
#
# The git ref is sanitized for semver pre-release identifier compatibility:
# only [0-9A-Za-z-] is allowed; all other characters are replaced with '-'.

if [[ $# -ne 4 ]]; then
	echo "Usage: $0 <cargo-toml-path> <git-ref> <commit-sha> <run-id>" >&2
	exit 1
fi

cargo_toml="$1"
git_ref="$2"
commit_sha="$3"
run_id="$4"

# Extract version from Cargo.toml via cargo metadata
version=$(cargo metadata --format-version 1 --no-deps --manifest-path "$cargo_toml" |
	jq -r '.packages[] | select(.name == "onshape-mcp") | .version')
if [[ -z "$version" ]]; then
	echo "ERROR: Could not extract version from $cargo_toml" >&2
	exit 1
fi

# Sanitize ref: replace any character not in [0-9A-Za-z-] with '-'
# Bash extglob: +([^0-9A-Za-z-]) matches one or more invalid characters
shopt -s extglob
sanitized_ref="${git_ref//+([^0-9A-Za-z-])/-}"

# Truncate commit SHA to 7 characters
short_sha="${commit_sha:0:7}"

staging_version="${version}-staging-${sanitized_ref}-${short_sha}-${run_id}"

echo "$staging_version"
