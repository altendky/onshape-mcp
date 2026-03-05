#!/usr/bin/env bash
# Resolve Rust versions ensuring consistency between rustup and Docker Hub
# Required environment variable: GRACE_PERIOD_HOURS

set -euo pipefail

# Function to get version from Rust release channel TOML
# Uses yq for proper TOML parsing to extract pkg.rust.version (not pkg.cargo.version)
get_rustup_version() {
	local channel="$1"
	# Parse the channel TOML and extract just the semver portion
	# e.g., "1.94.0-beta.2 (23a44d3c7 2026-01-25)" -> "1.94.0-beta.2"
	curl -sSf "https://static.rust-lang.org/dist/channel-rust-${channel}.toml" |
		yq -p toml '.pkg.rust.version' |
		grep -oP '^\S+'
}

# Function to get the release date from a Rust release channel TOML
# Returns: a date string like "2026-03-05"
get_rustup_release_date() {
	local channel="$1"
	curl -sSf "https://static.rust-lang.org/dist/channel-rust-${channel}.toml" |
		yq -p toml '.date'
}

# Function to check if Docker Hub has a specific rust image tag
# Returns: exit code 0 if tag exists, non-zero otherwise
check_docker_hub_tag() {
	local tag="$1"
	local url="https://hub.docker.com/v2/repositories/library/rust/tags/${tag}"
	curl -sSf "$url" >/dev/null 2>&1
}

# Function to find the latest available version for a given major.minor on Docker Hub
# Queries the Docker Hub API and filters for exact version-alpine tags
# Returns: the version string (e.g., "1.93.1") or empty if none found
# TODO: page_size=100 is Docker Hub's maximum; handle pagination if tag counts
#       per minor version ever exceed 100 (currently ~33 at most historically)
find_latest_docker_version() {
	local major="$1"
	local minor="$2"
	local prefix="${major}.${minor}."
	local url="https://hub.docker.com/v2/repositories/library/rust/tags?page_size=100&name=${prefix}"
	curl -sSf "$url" 2>/dev/null |
		jq -r '.results[].name' |
		grep -P "^${major}\.${minor}\.\d+-alpine$" |
		sed 's/-alpine$//' |
		sort -t. -k3 -n |
		tail -1
}

# Function to check if a timestamp is within the grace period
# Returns: exit code 0 if within grace period, 1 otherwise
is_within_grace_period() {
	local timestamp="$1"
	local grace_hours="$2"

	if [ -z "$timestamp" ]; then
		return 1
	fi

	# Convert ISO 8601 timestamp to epoch seconds
	local tag_epoch
	tag_epoch=$(date -d "$timestamp" +%s 2>/dev/null) || return 1

	local now_epoch
	now_epoch=$(date +%s)

	local grace_seconds=$((grace_hours * 3600))
	local age_seconds=$((now_epoch - tag_epoch))

	[ "$age_seconds" -le "$grace_seconds" ]
}

# Fetch stable channel TOML once, then parse both version and date from it.
# This avoids a TOCTOU race where two separate fetches could see different
# channel states if the TOML is updated between them (e.g., on release day).
echo "Querying rustup for stable channel..."
STABLE_TOML=$(curl -sSf "https://static.rust-lang.org/dist/channel-rust-stable.toml")
RUSTUP_STABLE=$(echo "$STABLE_TOML" | yq -p toml '.pkg.rust.version' | grep -oP '^\S+')
STABLE_RELEASE_DATE=$(echo "$STABLE_TOML" | yq -p toml '.date')
echo "Rustup stable: $RUSTUP_STABLE (released $STABLE_RELEASE_DATE)"

# Get beta version from rustup
echo "Querying rustup for beta version..."
RUSTUP_BETA=$(get_rustup_version "beta")
echo "Rustup beta: $RUSTUP_BETA"

# Check Docker Hub for stable version
echo "Checking Docker Hub for rust:${RUSTUP_STABLE}-alpine..."
if check_docker_hub_tag "${RUSTUP_STABLE}-alpine"; then
	echo "Docker Hub has rust:${RUSTUP_STABLE}-alpine"
	RESOLVED_STABLE="$RUSTUP_STABLE"
	STABLE_DOCKER_AVAILABLE="true"
else
	echo "::warning::Docker Hub does not have rust:${RUSTUP_STABLE}-alpine"
	STABLE_DOCKER_AVAILABLE="false"

	# STABLE_RELEASE_DATE was already parsed from the same TOML fetch as RUSTUP_STABLE.
	# If the stable was released recently (within the grace period), Docker Hub likely
	# hasn't caught up yet — this is expected on release day.
	echo "Rustup stable release date: $STABLE_RELEASE_DATE"

	# Find the latest previous version available on Docker Hub
	# by searching for the previous minor version's tags
	MAJOR=$(echo "$RUSTUP_STABLE" | cut -d. -f1)
	MINOR=$(echo "$RUSTUP_STABLE" | cut -d. -f2)
	PATCH=$(echo "$RUSTUP_STABLE" | cut -d. -f3)

	if [ "$PATCH" -gt 0 ]; then
		# Current version is a point release (e.g., 1.94.1)
		# Search the same minor for any available patch version
		SEARCH_MINOR="$MINOR"
	elif [ "$MINOR" -gt 0 ]; then
		# Current version is X.Y.0, search the previous minor
		SEARCH_MINOR=$((MINOR - 1))
	else
		echo "::error::Cannot compute previous version for ${RUSTUP_STABLE}"
		exit 1
	fi

	echo "Searching Docker Hub for latest ${MAJOR}.${SEARCH_MINOR}.* version..."
	PREV_VERSION=$(find_latest_docker_version "$MAJOR" "$SEARCH_MINOR")

	if [ -n "$PREV_VERSION" ]; then
		echo "Found previous version on Docker Hub: $PREV_VERSION"

		# Grace period is based on how recently the current stable was released,
		# not on when Docker Hub last updated the previous version.
		# On release day, Docker Hub needs time to build and publish new images.
		# Anchor to end-of-day since STABLE_RELEASE_DATE is date-only (no time).
		# This ensures the grace period is at least GRACE_PERIOD_HOURS from the
		# actual release moment, at the cost of up to ~24h extra tolerance.
		if is_within_grace_period "${STABLE_RELEASE_DATE}T23:59:59Z" "$GRACE_PERIOD_HOURS"; then
			echo "::warning::Using previous version ${PREV_VERSION} (stable ${RUSTUP_STABLE} released ${STABLE_RELEASE_DATE}, within ${GRACE_PERIOD_HOURS}h grace period)"
			RESOLVED_STABLE="$PREV_VERSION"
			STABLE_DOCKER_AVAILABLE="true"
		else
			echo "::error::Docker Hub lag exceeds grace period of ${GRACE_PERIOD_HOURS} hours"
			echo "::error::Stable ${RUSTUP_STABLE} was released ${STABLE_RELEASE_DATE} but Docker Hub still lacks rust:${RUSTUP_STABLE}-alpine"
			exit 1
		fi
	else
		echo "::error::Cannot find a suitable stable Rust version on Docker Hub"
		echo "::error::Rustup has ${RUSTUP_STABLE}, no ${MAJOR}.${SEARCH_MINOR}.*-alpine tags found on Docker Hub"
		exit 1
	fi
fi

# Beta doesn't have Docker images, so we just pass through the version
# It will be installed via rustup in the workflow
RESOLVED_BETA="$RUSTUP_BETA"

echo "Resolved versions:"
echo "  stable: $RESOLVED_STABLE"
echo "  beta: $RESOLVED_BETA"
echo "  stable-docker-available: $STABLE_DOCKER_AVAILABLE"

{
	echo "stable=$RESOLVED_STABLE"
	echo "beta=$RESOLVED_BETA"
	echo "stable-docker-available=$STABLE_DOCKER_AVAILABLE"
} >>"$GITHUB_OUTPUT"
