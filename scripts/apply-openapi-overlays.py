#!/usr/bin/env python3
"""Apply or check reviewed local guidance overlays on the vendored OpenAPI spec."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
SPEC_PATH = ROOT / "crates" / "onshape-mcp-io" / "onshape-openapi.json"
OVERLAYS_PATH = Path(__file__).with_name("openapi-overlays.json")
HTTP_METHODS = {"delete", "get", "head", "options", "patch", "post", "put", "trace"}


class OverlayError(Exception):
    """The spec or overlay manifest drifted from its reviewed structure."""


def resolve_target(spec: dict[str, Any], target: dict[str, str]) -> dict[str, Any]:
    kind = target.get("kind")
    if kind == "operation":
        operation_id = target["operationId"]
        matches = [
            operation
            for path_item in spec["paths"].values()
            for method, operation in path_item.items()
            if method.lower() in HTTP_METHODS
            and isinstance(operation, dict)
            and operation.get("operationId") == operation_id
        ]
        if len(matches) != 1:
            raise OverlayError(
                f"operationId {operation_id!r} resolved to {len(matches)} operations; expected 1"
            )
        return matches[0]
    if kind == "schema_property":
        schema_name = target["schema"]
        property_name = target["property"]
        try:
            return spec["components"]["schemas"][schema_name]["properties"][property_name]
        except KeyError as error:
            raise OverlayError(
                f"schema property {schema_name}.{property_name} no longer exists"
            ) from error
    raise OverlayError(f"unsupported overlay target kind: {kind!r}")


def target_label(target: dict[str, str]) -> str:
    if target["kind"] == "operation":
        return f"operationId {target['operationId']}"
    return f"schema property {target['schema']}.{target['property']}"


def process_overlays(check: bool) -> int:
    spec = json.loads(SPEC_PATH.read_text(encoding="utf-8"))
    manifest = json.loads(OVERLAYS_PATH.read_text(encoding="utf-8"))
    overlays = manifest.get("overlays")
    if not isinstance(overlays, list) or not overlays:
        raise OverlayError("overlay manifest must contain a nonempty 'overlays' array")

    changed = 0
    seen: set[tuple[str, str]] = set()
    for overlay in overlays:
        target = overlay["target"]
        field = overlay["field"]
        label = target_label(target)
        key = (json.dumps(target, sort_keys=True), field)
        if key in seen:
            raise OverlayError(f"duplicate overlay for {label} field {field!r}")
        seen.add(key)

        container = resolve_target(spec, target)
        current = container.get(field)
        desired = overlay["value"]
        if current == desired:
            continue
        if check:
            raise OverlayError(
                f"overlay missing for {label} field {field!r}; "
                "run 'python scripts/apply-openapi-overlays.py'"
            )
        if current != overlay.get("upstream"):
            raise OverlayError(
                f"upstream drift for {label} field {field!r}: found {current!r}; "
                f"expected reviewed upstream value {overlay.get('upstream')!r}"
            )
        container[field] = desired
        changed += 1

    if not check and changed:
        SPEC_PATH.write_text(
            json.dumps(spec, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    return changed


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify overlays are applied")
    args = parser.parse_args()
    try:
        changed = process_overlays(args.check)
    except (KeyError, TypeError, ValueError, OverlayError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        raise SystemExit(1) from error

    action = "Verified" if args.check else "Applied"
    print(f"{action} {len(json.loads(OVERLAYS_PATH.read_text())['overlays'])} overlays ({changed} changed)")


if __name__ == "__main__":
    main()
