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
#
# cargo metadata and jq are run separately so that set -e catches
# failures in either command (process substitution does not propagate
# exit codes).
metadata=$(cargo metadata --format-version 1 --no-deps)

crate_lines=$(printf '%s\n' "$metadata" | jq -r '
    [.packages[] | select(.publish == null or .publish == [])] as $publishable |
    ($publishable | map(.name)) as $ws_names |
    $publishable[] |
    [.name] + [.dependencies[] | select(.kind == null) | .name | select(. as $n | $ws_names | index($n))] |
    join(" ")
  ')

readarray -t crate_info <<<"$crate_lines"

# readarray on an empty string produces a single empty-string element;
# treat that the same as "no crates found".
if [[ ${#crate_info[@]} -eq 1 && -z "${crate_info[0]}" ]]; then
	crate_info=()
fi

if ((${#crate_info[@]} == 0)); then
	echo "ERROR: No publishable workspace crates found" >&2
	exit 1
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
				if publish_output=$(cargo publish -p "$name" 2>&1 | tee /dev/stderr); then
					break
				fi
				if echo "$publish_output" | grep -q 'already exists'; then
					echo "  ${name} already published, skipping."
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
