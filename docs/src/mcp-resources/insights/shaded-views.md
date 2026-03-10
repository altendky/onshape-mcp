# Part Studio Shaded Views

How to get rendered images of a Part Studio via the Onshape API.

## Preferred: `onshape_screenshot` Tool

Use the `onshape_screenshot` tool instead of calling the shaded views endpoint directly.
It handles `pixelSize=0` (auto-fit), view matrix computation, base64 decoding, and file
saving automatically. Provide document/element IDs, a view spec (named preset like
`"front"` or `"isometric"`, or custom azimuth/elevation angles), and an output file path.
The tool saves the PNG and returns the file path — no base64 handling needed.

Call the tool multiple times for multiple views.

The rest of this document explains the underlying API for reference.

## Raw API Endpoint

Use `getPartStudioShadedViews` (`GET /partstudios/d/{did}/{wvm}/{wvmid}/e/{eid}/shadedviews`)
to render a Part Studio server-side. The response is a JSON object with an `images` array
containing **base64-encoded PNG strings**. Because the payload is JSON, this works through
JSON-based API tools (unlike the thumbnail endpoints — see [Alternatives](#alternatives) below).

## Critical Parameter: `pixelSize`

The `pixelSize` parameter controls how many meters each pixel represents.

| Value | Behavior |
| ------- | ---------- |
| `0.003` (default) | 3 mm/pixel — at 500×500 output this creates a 1.5 m × 1.5 m viewport |
| `0` | **Auto-fit** — sizes the viewport to the part's bounding box |
| any positive number | Fixed scale in meters per pixel |

**Set `pixelSize=0` for almost all use cases.** The default produces a 1.5 m viewport,
which makes most parts appear tiny in the rendered image.

## View Matrix

The `viewMatrix` parameter accepts either a **named preset** or a **12-number
comma-separated matrix** (3 rows × 4 columns).

### Named Presets

`front`, `back`, `top`, `bottom`, `left`, `right`

### Custom Matrix

The matrix maps model coordinates to view coordinates:

- Columns 1–3: rotation (model axes → view axes)
- Column 4: translation (in meters)
- View coordinates: x right, y up, z toward viewer
- Model front view: x right, y away from viewer, z up

**Approximate isometric:**

```text
0.612,0.612,0,0,-0.354,0.354,0.707,0,0.707,-0.707,0.707,0
```

The identity matrix `1,0,0,0,0,1,0,0,0,0,1,0` corresponds to the top view.
The first three columns should be orthonormal with a positive determinant.

## Other Parameters

| Parameter | Type | Default | Description |
| --------- | ---- | ------- | ----------- |
| `outputHeight` | integer | 500 | Image height in pixels |
| `outputWidth` | integer | 500 | Image width in pixels |
| `edges` | string | `show` | `show` or `hide` visible edges |
| `useAntiAliasing` | boolean | `false` | Smooth model boundaries (costs performance) |
| `showAllParts` | boolean | `false` | Show all parts regardless of user visibility settings |
| `includeSurfaces` | boolean | `false` | Include surfaces (only when `showAllParts` is true) |
| `includeWires` | boolean | `false` | Include wire bodies |

## Alternatives

### Thumbnail endpoints

`getElementThumbnail` returns metadata with URLs to pre-rendered thumbnails at fixed
sizes (70×40, 300×170, 300×300, 600×340). The actual image endpoint
`getElementThumbnailWithSize` returns raw `image/png` binary, which causes **HTTP 406**
through JSON-only API tools that send `Accept: application/json`. See
[#123](https://github.com/altendky/onshape-mcp/issues/123) for tracking.

### 3D export endpoints

`exportPartStudioGltf`, `exportPartStudioStl`, etc. export geometry data, not rendered
images. Useful for other purposes but not for quick visual inspection.

## Pitfalls

1. **Forgetting `pixelSize=0`** — The default `0.003` almost always produces a
   mostly-empty image with the part rendered as a tiny speck.

2. **Anti-aliasing on large images** — `useAntiAliasing=true` can cause server-side
   failures for high-resolution requests due to memory usage.

3. **Large base64 strings** — A 500×500 auto-fit render produces a substantial base64
   string. Avoid passing it as a shell argument (argument length limits). Write to a
   file first, then decode. The `onshape_screenshot` tool handles this automatically.

4. **Sketch-only Part Studios render as blank images** — The shaded views API
   renders solid and surface bodies. Sketch geometry (lines, arcs, points,
   constraints) is not rendered. A Part Studio containing only sketches and no
   solid features will produce a blank white image regardless of `showAllParts`,
   `includeWires`, or `includeSurfaces` settings. To verify sketch geometry
   before creating solid features, use `evalFeatureScript` to inspect face areas
   and edge endpoints (see the [Sketch](sketch.md) insight's "Verifying Sketch
   Geometry" section).
