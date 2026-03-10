# FeatureScript

How to write, deploy, and debug FeatureScript code in Onshape Feature Studios
via the API.

## FeatureScript Version Discovery

The FeatureScript version must match the version deployed on the Onshape
instance. The documentation site (`/FsDoc/`) may show a version that is not yet
deployed. To find the correct version, query the standard library source
document:

```text
GET /api/v10/featurestudios/d/12312312345abcabcabcdeff/w/a855e4161c814f2e9ab3698a/e/{elementId}
```

where `{elementId}` is the element ID of any stdlib module (e.g., `common.fs`).
The first line of the response `contents` field gives the current version:

```text
FeatureScript 2892;
```

### Finding stdlib element IDs

List all Feature Studio elements in the standard library document:

```text
GET /api/v10/documents/d/12312312345abcabcabcdeff/w/a855e4161c814f2e9ab3698a/elements?elementType=FEATURESTUDIO
```

This returns an array of `{ "name": "common.fs", "id": "..." }` objects. Use
any element's `id` in the `getFeatureStudioContents` call above.

### Standard library document coordinates

| Field | Value |
| ----- | ----- |
| Document ID | `12312312345abcabcabcdeff` |
| Workspace ID | `a855e4161c814f2e9ab3698a` |
| Owner | Onshape (system document) |

These are stable identifiers — the standard library document does not change
location.

## Import Conventions

A Feature Studio starts with a version declaration and an import:

```featurescript
FeatureScript 2892;
import(path : "onshape/std/geometry.fs", version : "2892.0");
```

The version in the import **must** use the `"NNNN.0"` format matching the
`FeatureScript NNNN;` declaration.

### `geometry.fs` vs `common.fs`

- `geometry.fs` — imports **all** standard library features and functions
  (recommended)
- `common.fs` — imports a common subset, slightly faster to compile

### Individual modules are NOT separately importable

Modules like `extend.fs`, `projectCurves.fs`, and `mutualTrim.fs` are
**included transitively** through `geometry.fs` or `common.fs`. Importing them
directly will fail:

```featurescript
// WRONG — these produce "Module not found" errors:
import(path : "onshape/std/extend.fs", version : "2892.0");
import(path : "onshape/std/projectCurves.fs", version : "2892.0");
```

All standard library operations (`opLoft`, `opSplitFace`, `opFitSpline`,
`opFaceBlend`, etc.) are available through a single `geometry.fs` import.

## Reading and Writing Feature Studio Code

### Read current code

Use the `getFeatureStudioContents` endpoint (via MCP: `onshape_api_call` with
endpoint `getFeatureStudioContents`):

**Path params:** `did`, `wvm` (`"w"`), `wvmid`, `eid`

The response includes a `contents` field with the full FeatureScript source.

### Write new code

Use `updateFeatureStudioContents`:

**Path params:** same as above

**Body:**

```json
{ "contents": "FeatureScript 2892;\nimport(path : ...);\n..." }
```

The response includes the updated `contents` and a new `sourceMicroversion`.
Compilation happens server-side; errors appear in the FeatureScript notices
panel (see below).

## FeatureScript Documentation

| Resource | URL |
| -------- | --- |
| Library reference | `https://cad.onshape.com/FsDoc/library.html` |
| Language guide | `https://cad.onshape.com/FsDoc/intro.html` |
| Feature UI spec | `https://cad.onshape.com/FsDoc/uispec.html` |
| Tutorials | `https://cad.onshape.com/FsDoc/tutorials/create-a-slot-feature.html` |
| Imports guide | `https://cad.onshape.com/FsDoc/imports.html` |
| Debugging guide | `https://cad.onshape.com/FsDoc/debugging-in-feature-studios.html` |

The library reference is large. When fetching it, consider searching for
specific function names rather than reading the entire page.

## Accessing FeatureScript Notices (Workaround)

**The Onshape API does not expose Feature Studio compilation errors or
warnings.** After writing code via `updateFeatureStudioContents`, you cannot
programmatically check whether it compiled successfully through the API alone.

### Playwright workaround

Use a browser automation tool (e.g., Playwright) to read the notices panel:

1. **Navigate** to the Feature Studio URL:

   ```text
   https://cad.onshape.com/documents/{did}/w/{wid}/e/{eid}
   ```

2. **Open the notices panel** by clicking the toggle button:

   ```javascript
   document.querySelector('.notice-pane-toggle-button').click();
   ```

3. **Read all notices** by finding the panel content:

   ```javascript
   const allDivs = document.querySelectorAll('div');
   for (const div of allDivs) {
     if (div.textContent.includes('FeatureScript notices') && div.children.length > 2) {
       return div.innerText;
     }
   }
   ```

### Notice types in the editor gutter

| CSS class | Appearance | Meaning |
| --------- | ---------- | ------- |
| `fs-notice-error` | Red X | Compilation error |
| `fs-notice-warning` | Yellow triangle | Warning (unused variable, module not found, etc.) |

### Checking for clean compilation

If the notices panel shows only `"Result: Regeneration complete"` with no
error/warning tables, the code compiled successfully.

### evalFeatureScript workaround

If the Feature Studio is imported by a Part Studio, compilation errors surface
in the `notices` array of an `evalFeatureScript` call on that Part Studio. Run
any trivial script (the content does not matter — compilation of imported
modules happens regardless):

```text
POST /partstudios/d/{did}/w/{wid}/e/{eid}/featurescript

{
  "script": "function(context is Context, queries is map) { return {}; }"
}
```

The response `notices` array contains `BTNotice-227` objects with:

| Field | Example |
| ----- | ------- |
| `level` | `"WARNING"` or `"ERROR"` |
| `message` | `"Function evaluateQuery with 2 argument(s) not found"` |
| `stackTrace[].document` | Feature Studio element ID where the error occurs |
| `stackTrace[].line` | Line number (0-indexed) |
| `stackTrace[].column` | Column number (0-indexed) |

**Limitations:** This requires a Part Studio in the same document that imports
the Feature Studio. It surfaces compilation notices indirectly — there is still
no direct API to query a Feature Studio's compilation status.

## Pitfalls

1. **Version mismatch is silent and catastrophic** — If the FeatureScript
   version doesn't exist on the instance, every symbol from the standard
   library becomes unresolved. The error cascade (dozens of "Variable not
   found", "Function not found", "No declaration for type" errors) obscures
   the root cause. Always verify the version first.

2. **FsDoc version may differ from deployed version** — The documentation at
   `/FsDoc/` is generated from a specific build. It may reference a newer
   version than what is currently deployed. Always discover the version from
   the standard library document.

3. **No API for compilation status** — After pushing code, you must use the
    browser workaround to verify compilation. Plan for this in your workflow.
    Note: *runtime* feature errors (errors during Part Studio regeneration) **can**
    be retrieved via the API — see [Part Studio](part-studio.md).

4. **The standard library source is readable** — When the `/FsDoc/` reference
   is insufficient, you can read the actual source of any stdlib module via
   `getFeatureStudioContents` on the stdlib document. This is useful for
   checking exact function signatures, available enum values, or understanding
   how built-in features work.
