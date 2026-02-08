#!/usr/bin/env bash
# Publish all workspace crates to crates.io in dependency order.
#
# Uses cargo metadata to discover workspace members and their
# inter-workspace dependencies, then topologically sorts and
# publishes leaves first.
#
# Crates with publish = false (or restricted registries) are skipped.

set -euo pipefail

# Get workspace package names, their workspace-internal dependencies,
# and whether they are publishable.
# Output format: one line per publishable crate, "name dep1 dep2 ..."
readarray -t crate_info < <(
	cargo metadata --format-version 1 --no-deps | jq -r '
    .packages | map(.name) as $ws_names |
    .[] |
    select(.publish == null or .publish == []) |
    [.name] + [.dependencies[] | .name | select(. as $n | $ws_names | index($n))] |
    join(" ")
  '
)

if ((${#crate_info[@]} == 0)); then
	echo "No publishable workspace crates found"
	exit 0
fi

echo "Found ${#crate_info[@]} publishable crates"

is_published() {
	local target="$1"
	for item in "${published[@]+"${published[@]}"}"; do
		if [[ "$item" == "$target" ]]; then
			return 0
		fi
	done
	return 1
}

published=()
while ((${#crate_info[@]} > 0)); do
	progress=false
	next=()

	for entry in "${crate_info[@]}"; do
		read -ra parts <<<"$entry"
		name="${parts[0]}"
		deps=("${parts[@]:1}")

		# Check if all workspace deps have been published
		all_met=true
		for dep in "${deps[@]+"${deps[@]}"}"; do
			if ! is_published "$dep"; then
				all_met=false
				break
			fi
		done

		if $all_met; then
			echo "Publishing ${name}..."
			max_retries=3
			retry_delay=15
			for attempt in $(seq 1 "$max_retries"); do
				if cargo publish -p "$name"; then
					break
				fi
				if [[ "$attempt" -eq "$max_retries" ]]; then
					echo "ERROR: cargo publish failed for ${name} after ${max_retries} attempts" >&2
					exit 1
				fi
				echo "Retrying in ${retry_delay}s (attempt ${attempt}/${max_retries})..."
				sleep "$retry_delay"
				retry_delay=$((retry_delay * 2))
			done
			published+=("$name")
			progress=true

			# Brief delay for crates.io index propagation.  Skip only when
			# this round started with a single crate (meaning nothing else
			# will be published after it).  When multiple crates remain at
			# the start of the round, we always sleep because later
			# iterations — in this round or the next — may need the index
			# to reflect what was just published.
			if ((${#crate_info[@]} > 1)); then
				sleep 15
			fi
		else
			next+=("$entry")
		fi
	done

	if ! $progress; then
		echo "ERROR: Cannot resolve publish order. Remaining:" >&2
		printf '  %s\n' "${next[@]}" >&2
		exit 1
	fi

	crate_info=("${next[@]+"${next[@]}"}")
done

echo "Published ${#published[@]} crates: ${published[*]}"
