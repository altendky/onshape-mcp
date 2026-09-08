#!/usr/bin/env bash

set -euo pipefail

if (($# != 2)); then
	echo "Usage: $0 <tarball> <dist-tag>" >&2
	exit 2
fi

tarball=$1
dist_tag=$2

if [[ ! -f "$tarball" ]]; then
	echo "ERROR: Tarball not found: $tarball" >&2
	exit 1
fi

metadata=$(npm pack --dry-run --json "$tarball")
name=$(jq -r '.[0].name // empty' <<<"$metadata")
version=$(jq -r '.[0].version // empty' <<<"$metadata")
local_integrity=$(jq -r '.[0].integrity // empty' <<<"$metadata")

if [[ -z "$name" || -z "$version" || -z "$local_integrity" ]]; then
	echo "ERROR: Could not read package metadata from $tarball" >&2
	exit 1
fi

package_version="${name}@${version}"

registry_integrity() {
	local output
	local status

	set +e
	output=$(npm view "$package_version" dist.integrity --json 2>&1)
	status=$?
	set -e

	if ((status == 0)); then
		jq -r 'select(type == "string")' <<<"$output"
		return 0
	fi

	if [[ "$output" == *"E404"* ]]; then
		return 1
	fi

	printf '%s\n' "$output" >&2
	return 2
}

set +e
remote_integrity=$(registry_integrity)
lookup_status=$?
set -e

if ((lookup_status == 0)); then
	if [[ "$remote_integrity" != "$local_integrity" ]]; then
		echo "ERROR: Published integrity does not match $tarball for $package_version" >&2
		echo "  local:  $local_integrity" >&2
		echo "  remote: $remote_integrity" >&2
		exit 1
	fi
	remote_tag=$(npm view "$name" "dist-tags.$dist_tag" --json | jq -r '.')
	if [[ "$remote_tag" != "$version" ]]; then
		echo "ERROR: Dist-tag $dist_tag for $name points to $remote_tag, expected $version" >&2
		exit 1
	fi
	echo "$package_version is already published with matching integrity; skipping."
	exit 0
elif ((lookup_status != 1)); then
	exit "$lookup_status"
fi

echo "Publishing $tarball as $package_version with dist-tag $dist_tag..."
set +e
publish_output=$(npm publish "$tarball" --tag "$dist_tag" --access public 2>&1)
publish_status=$?
set -e
printf '%s\n' "$publish_output"

# npm may lose the response after the registry accepted the package. Polling
# also handles normal registry propagation and closes the check/publish race.
for attempt in $(seq 1 15); do
	set +e
	remote_integrity=$(registry_integrity)
	lookup_status=$?
	set -e

	if ((lookup_status == 0)); then
		if [[ "$remote_integrity" != "$local_integrity" ]]; then
			echo "ERROR: Published integrity does not match $tarball for $package_version" >&2
			exit 1
		fi
		remote_tag=$(npm view "$name" "dist-tags.$dist_tag" --json | jq -r '.')
		if [[ "$remote_tag" == "$version" ]]; then
			echo "$package_version is available with matching integrity and dist-tag."
			exit 0
		fi
	elif ((lookup_status != 1)); then
		exit "$lookup_status"
	fi

	if ((attempt < 15)); then
		sleep 10
	fi
done

if ((publish_status != 0)); then
	echo "ERROR: npm publish failed for $package_version" >&2
	exit "$publish_status"
fi

echo "ERROR: $package_version did not appear with the expected integrity and dist-tag" >&2
exit 1
