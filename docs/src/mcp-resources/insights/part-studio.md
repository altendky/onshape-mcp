# Part Studio

General patterns for working with Part Studio features, querying state, and
debugging errors via the API.

## Retrieving Feature Runtime Errors

The `getPartStudioFeatures` endpoint returns a `featureStates` map, but the
`BTFeatureState-1688` schema only exposes `featureStatus` (OK, ERROR, INFO) and
`inactive` (boolean). It does **not** include the error message text.

To retrieve the actual error message for a failed feature, use
`evalFeatureScript` with the `getFeatureStatus()` function:

```json
{
  "script": "function(context is Context, queries is map) { return getFeatureStatus(context, makeId(\"<featureId>\")); }",
  "serializationVersion": "1.2.16",
  "sourceMicroversion": "<current microversion>"
}
```

Replace `<featureId>` with the feature's ID (e.g., `F6PKLet2t74iKQ4_1`), found
in the `featureId` field of each feature in the `getPartStudioFeatures`
response.

### Response

The result is a `FeatureStatus` map with four fields:

| Field | Type | Example |
| ----- | ---- | ------- |
| `statusType` | StatusType enum | `"ERROR"`, `"INFO"`, `"OK"` |
| `statusEnum` | ErrorStringEnum | `"CUSTOM_ERROR"`, `"FILLET_FAILED"`, etc. |
| `statusMsg` | string | `"Face A split failed: loft edge may not fully cross face."` |
| `faultyParameters` | string array | `["edges"]` |

For custom FeatureScript features, `statusEnum` is typically `CUSTOM_ERROR` and
`statusMsg` contains the message from `regenError()` or `throw`. For built-in
features, `statusEnum` is a specific error constant (e.g., `FILLET_FAILED`,
`SWEEP_PATH_FAILED`) but `statusMsg` is **not returned** — only `statusEnum`,
`statusType`, and `faultyParameters` are present. Use the `onshape_error_lookup`
tool to resolve built-in `statusEnum` values to human-readable messages.

### Checking multiple features

To check all features at once, first call `getPartStudioFeatures` to get the
feature list and identify which features have `featureStatus: "ERROR"`, then
call `evalFeatureScript` for each one to retrieve the error message.

### Example: querying a failing feature

```text
POST /partstudios/d/{did}/w/{wid}/e/{eid}/featurescript

{
  "script": "function(context is Context, queries is map) { return getFeatureStatus(context, makeId(\"F6PKLet2t74iKQ4_1\")); }"
}
```

Response (in `result`):

```json
{
  "statusType": "ERROR",
  "statusEnum": "CUSTOM_ERROR",
  "statusMsg": "Face A split failed: loft edge may not fully cross face.",
  "faultyParameters": ["edges"]
}
```

### Compilation notices vs runtime errors

This technique retrieves **runtime** feature errors (errors that occur when a
feature executes during Part Studio regeneration). For **compilation** errors
in Feature Studio code, see the [FeatureScript insight](featurescript.md).
