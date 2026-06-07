# OpenAPI Updates

The Onshape OpenAPI specification is vendored for reference and for the generic
API tools. Updating it is a human review process: replacing the JSON is only the
mechanical first step.

| Setting | Value |
| ------- | ----- |
| Location | `crates/onshape-mcp-io/onshape-openapi.json` |
| Source | `https://cad.onshape.com/api/v6/openapi` |
| License | Apache 2.0 (see `crates/onshape-mcp-io/ONSHAPE-API-LICENSE`) |
| Format | Pretty-printed JSON |

## Manual Review Checklist

Each OpenAPI update should review code and docs that rely on specific operation
IDs, schema names, paths, or request/response shapes.

### Fetch And Replace Spec

- Download `https://cad.onshape.com/api/v6/openapi`
- Pretty-print it into `crates/onshape-mcp-io/onshape-openapi.json`
- Confirm the source and license are still covered by
  `crates/onshape-mcp-io/ONSHAPE-API-LICENSE`

### Summarize Spec Drift

- Record old and new `info.version`
- Record old and new `servers[0].url`
- Compare operation count and schema count
- List added and removed operation IDs
- List added and removed schema names when relevant

### Review Generic OpenAPI Tooling

- Verify `onshape-openapi` still parses the spec
- Verify the generic tools still work conceptually: `onshape_api_search`,
  `onshape_api_explain`, `onshape_api_schema`, and `onshape_api_call`
- Check tool descriptions and docs for examples that mention operation IDs or
  schema names, and update any that no longer exist

### Review Hard-Coded MCP Wrappers

- Verify `onshape_screenshot` still matches `getPartStudioShadedViews`
- Confirm hard-coded operation IDs, path parameters, query parameter names, and
  response handling still match the spec

### Review Typed Endpoint Helpers

- Review `crates/onshape-client-core/src/endpoints/`
- For each wrapper, confirm the operation ID still exists
- Confirm the method and path template still match
- Confirm query and header parameters still match
- Confirm request content type still matches
- Confirm referenced request and response schemas still have the expected fields
- Pay special attention to optional field additions that should be exposed in
  typed request structs

### Update Version-Specific References

- Update docs and code examples that describe the current API base path
- Preserve historical verification notes unless the behavior is re-tested

### Run Verification

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo nextest run --all-features`

## Automation Follow-Up

`.github/workflows/update-openapi-spec.yml` is planned but not currently
implemented. If implemented later, the workflow should still leave human review
for typed endpoint helpers and hard-coded wrappers, because spec replacement
alone is not enough.
