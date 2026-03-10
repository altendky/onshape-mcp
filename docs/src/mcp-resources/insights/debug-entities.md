# Debug Entities

How to access FeatureScript debug entity information via the API, and how to
create API-visible debug geometry for use with `onshape_screenshot`.

## Background

FeatureScript's `debug()` function and related helpers (`addDebugEntities`,
`addDebugPoint`, `addDebugLine`, `addDebugArrow`) create colored visual
overlays in the Onshape web UI. These overlays are:

- **Only visible when the feature's edit dialog is open** in the browser
- **Not real model entities** — they cannot be queried
- **Not rendered by the server-side shaded views API** — `onshape_screenshot`
  will never show them

This means an API-only workflow (no browser) cannot see `debug()` output.
Two complementary approaches address this gap.

## Approach 1: Data Extraction via `evalFeatureScript`

Instead of viewing debug visuals, extract the same geometric data that
`debug()` would display. Use `evalFeatureScript` to run FeatureScript lambdas
that call standard library evaluation functions.

### Basic Pattern

```text
POST /partstudios/d/{did}/w/{wid}/e/{eid}/featurescript

{
  "script": "function(context is Context, queries is map) { ... return result; }"
}
```

The response `result` field contains the returned value as structured JSON.

### Common Extraction Snippets

#### Face tangent plane at a point on an edge

Equivalent of `debug(context, tangentPlane, DebugColor.RED)`:

```featurescript
function(context is Context, queries is map) {
    var edge = queries["edge"];
    var faces = evaluateQuery(context, qAdjacent(edge, AdjacencyType.EDGE, EntityType.FACE));
    var plane = evFaceTangentPlaneAtEdge(context, {
            "edge" : edge,
            "face" : faces[0],
            "parameter" : 0.5
    });
    return {
        "origin" : plane.origin / meter,
        "normal" : plane.normal,
        "x" : plane.x
    };
}
```

Pass the edge via the `queries` map:

```json
{
  "queries": { "edge": ["transient-id-string"] }
}
```

#### Edge tangent line at a parameter

Equivalent of `debug(context, tangentLine, DebugColor.ORANGE)`:

```featurescript
function(context is Context, queries is map) {
    var edge = queries["edge"];
    var tangent = evEdgeTangentLine(context, {
            "edge" : edge,
            "parameter" : 0.5
    });
    return {
        "origin" : tangent.origin / meter,
        "direction" : tangent.direction
    };
}
```

#### Count and list entities matching a query

Equivalent of `debug(context, someQuery, DebugColor.GREEN)` to understand
what entities match:

```featurescript
function(context is Context, queries is map) {
    var allFaces = evaluateQuery(context, qEverything(EntityType.FACE));
    return {
        "count" : size(allFaces),
        "ids" : transientQueriesToStrings(allFaces)
    };
}
```

#### Edge endpoint coordinates

Get the 3D positions of an edge's endpoints:

```featurescript
function(context is Context, queries is map) {
    var edge = queries["edge"];
    var start = evEdgeTangentLine(context, { "edge" : edge, "parameter" : 0.0 });
    var end = evEdgeTangentLine(context, { "edge" : edge, "parameter" : 1.0 });
    return {
        "start" : start.origin / meter,
        "end" : end.origin / meter
    };
}
```

#### Bounding box (tight)

```featurescript
function(context is Context, queries is map) {
    var box = evBox3d(context, {
            "topology" : qAllModifiableSolidBodies(),
            "tight" : true
    });
    return {
        "minCorner" : box.minCorner / meter,
        "maxCorner" : box.maxCorner / meter
    };
}
```

#### Face curvature at a point

For debugging curvature-continuous (G2) features:

```featurescript
function(context is Context, queries is map) {
    var face = queries["face"];
    var curvature = evFaceCurvature(context, {
            "face" : face,
            "parameter" : vector(0.5, 0.5)
    });
    return {
        "minCurvature" : curvature.minCurvature * meter,
        "maxCurvature" : curvature.maxCurvature * meter,
        "minDirection" : curvature.minDirection,
        "maxDirection" : curvature.maxDirection
    };
}
```

### Unit Handling

FeatureScript values with units cannot be directly serialized. Divide by the
appropriate unit to get a dimensionless number:

| Type | Conversion | Example |
| ---- | ---------- | ------- |
| Length | `/ meter` | `point / meter` → `[0.05, 0.0, 0.025]` |
| Area | `/ (meter * meter)` | `area / (meter * meter)` → `0.0025` |
| Angle | `/ radian` or `/ degree` | `angle / degree` → `45.0` |
| Curvature | `* meter` | `curvature * meter` → `10.0` |

Unitless vectors (directions, normals) need no conversion.

### Limitations

- **Only lambda expressions** are supported — you cannot call custom feature
  functions (like `computeCrossSectionFrame`) directly unless they are exported
  from an imported Feature Studio.
- **Query resolution** depends on model state — pass transient query strings
  obtained from a prior `evalFeatureScript` call or use deterministic queries
  like `qCreatedBy(makeId("featureId"), EntityType.EDGE)`.
- **Returns data, not visuals** — you get numbers that require interpretation.
  For spatial relationships, multiple queries from different viewpoints may be
  needed.

## Approach 2: Dual-Mode Debug Helpers

A set of reusable helper functions that present a single debug API. Each
helper takes a `wireMode` boolean: when `false` it delegates to the standard
`debug()` / `addDebugLine` / etc. (for the user in the browser); when `true`
it creates a real wire body that is visible in `onshape_screenshot`.

### Design Principles

1. **One call site, two modes** — feature code calls one function per debug
   visualization. The `wireMode` flag controls the output. No duplicate
   code paths.
2. **Regular `debug()` is the default** — `wireMode` is generally `false`.
   The user sees colored overlays when editing the feature interactively.
3. **Wire bodies are for the agent** — the agent temporarily enables
   `wireMode` via a feature parameter toggle, takes a screenshot, then
   disables it.
4. **Bodies are named by color and label** — since wire bodies have no color
   in shaded views, each wire body is named `"debug-{COLOR}-{label}"` via
   `setProperty`. The agent can identify what each wire represents by
   querying body names.
5. **Helpers live in a separate Feature Studio** — named `debug-wire-helpers`,
   importable by any custom feature.

### Feature Studio: `debug-wire-helpers`

```featurescript
FeatureScript NNNN;
import(path : "onshape/std/geometry.fs", version : "NNNN.0");

/**
 * Internal helper: set the body name to encode color and label.
 */
function nameDebugBody(context is Context, body is Query,
        color is DebugColor, label is string)
{
    setProperty(context, {
            "entities" : body,
            "propertyType" : PropertyType.NAME,
            "value" : "debug-" ~ color ~ "-" ~ label
    });
}

/**
 * Draw a line segment between two 3D points.
 *
 * wireMode false: calls addDebugLine (colored overlay, edit dialog only).
 * wireMode true:  creates a wire body named "debug-{color}-{label}".
 */
export function debugWireLine(context is Context, id is Id,
        wireMode is boolean,
        point1 is Vector, point2 is Vector,
        color is DebugColor, label is string)
{
    if (wireMode)
    {
        opFitSpline(context, id, { "points" : [point1, point2] });
        nameDebugBody(context, qCreatedBy(id, EntityType.BODY), color, label);
    }
    else
    {
        addDebugLine(context, point1, point2, color);
    }
}

/**
 * Draw a spline through an array of 3D points.
 *
 * wireMode false: calls addDebugLine for consecutive point pairs.
 * wireMode true:  creates a wire body spline named "debug-{color}-{label}".
 */
export function debugWireSpline(context is Context, id is Id,
        wireMode is boolean,
        points is array,
        color is DebugColor, label is string)
{
    if (wireMode)
    {
        opFitSpline(context, id, { "points" : points });
        nameDebugBody(context, qCreatedBy(id, EntityType.BODY), color, label);
    }
    else
    {
        for (var i = 0; i < size(points) - 1; i += 1)
        {
            addDebugLine(context, points[i], points[i + 1], color);
        }
    }
}

/**
 * Mark a 3D point.
 *
 * wireMode false: calls addDebugPoint (colored dot, edit dialog only).
 * wireMode true:  creates a small cross-shaped wire marker named
 *                 "debug-{color}-{label}".
 *
 * `size` controls the arm length of the cross (e.g., 1 * millimeter).
 */
export function debugWirePoint(context is Context, id is Id,
        wireMode is boolean,
        point is Vector, size is ValueWithUnits,
        color is DebugColor, label is string)
{
    if (wireMode)
    {
        opFitSpline(context, id + "x", {
                "points" : [
                    point - vector(size, 0 * meter, 0 * meter),
                    point + vector(size, 0 * meter, 0 * meter)
                ]
        });
        opFitSpline(context, id + "y", {
                "points" : [
                    point - vector(0 * meter, size, 0 * meter),
                    point + vector(0 * meter, size, 0 * meter)
                ]
        });
        opFitSpline(context, id + "z", {
                "points" : [
                    point - vector(0 * meter, 0 * meter, size),
                    point + vector(0 * meter, 0 * meter, size)
                ]
        });
        nameDebugBody(context, qCreatedBy(id, EntityType.BODY), color, label);
    }
    else
    {
        addDebugPoint(context, point, color);
    }
}

/**
 * Highlight existing entities (faces, edges, vertices, bodies).
 *
 * wireMode false: calls addDebugEntities (colored highlight, edit dialog only).
 * wireMode true:  extracts wire copies of edges from the queried entities,
 *                 named "debug-{color}-{label}". For faces, extracts their
 *                 bounding edges.
 */
export function debugWireEntities(context is Context, id is Id,
        wireMode is boolean,
        entities is Query,
        color is DebugColor, label is string)
{
    if (wireMode)
    {
        // Extract edges from whatever entities are provided
        var edges = qEntityFilter(entities, EntityType.EDGE);
        var faceEdges = qAdjacent(
                qEntityFilter(entities, EntityType.FACE),
                AdjacencyType.EDGE, EntityType.EDGE);
        var allEdges = qUnion([edges, faceEdges]);
        if (size(evaluateQuery(context, allEdges)) > 0)
        {
            opExtractWires(context, id, { "edges" : allEdges });
            nameDebugBody(context,
                    qCreatedBy(id, EntityType.BODY), color, label);
        }
    }
    else
    {
        addDebugEntities(context, entities, color);
    }
}

/**
 * Draw an arrow from `from` to `to`.
 *
 * wireMode false: calls addDebugArrow (colored arrow, edit dialog only).
 * wireMode true:  creates wire bodies for shaft + arrowhead lines, named
 *                 "debug-{color}-{label}".
 *
 * `headSize` controls the arrowhead length (e.g., 1 * millimeter).
 */
export function debugWireArrow(context is Context, id is Id,
        wireMode is boolean,
        from is Vector, to is Vector, headSize is ValueWithUnits,
        color is DebugColor, label is string)
{
    if (wireMode)
    {
        var dir = normalize(to - from);
        var perp = perpendicularVector(dir);
        var tip1 = to - headSize * dir + headSize * 0.3 * perp;
        var tip2 = to - headSize * dir - headSize * 0.3 * perp;

        opFitSpline(context, id + "shaft", { "points" : [from, to] });
        opFitSpline(context, id + "head1", { "points" : [to, tip1] });
        opFitSpline(context, id + "head2", { "points" : [to, tip2] });
        nameDebugBody(context, qCreatedBy(id, EntityType.BODY), color, label);
    }
    else
    {
        addDebugArrow(context, from, to, headSize, color);
    }
}

/**
 * Visualize a plane as a rectangle with a normal arrow.
 *
 * wireMode false: calls debug(context, plane, color).
 * wireMode true:  creates wire bodies for a rectangle in the plane plus
 *                 a normal arrow, named "debug-{color}-{label}".
 *
 * `size` controls the half-extent of the rectangle.
 */
export function debugWirePlane(context is Context, id is Id,
        wireMode is boolean,
        plane is Plane, size is ValueWithUnits,
        color is DebugColor, label is string)
{
    if (wireMode)
    {
        var yDir = yAxis(plane);
        var c1 = plane.origin + size * plane.x + size * yDir;
        var c2 = plane.origin - size * plane.x + size * yDir;
        var c3 = plane.origin - size * plane.x - size * yDir;
        var c4 = plane.origin + size * plane.x - size * yDir;
        opFitSpline(context, id + "rect", {
                "points" : [c1, c2, c3, c4, c1]
        });
        // Normal indicator
        var normalTip = plane.origin + size * plane.normal;
        var normalDir = normalize(plane.normal);
        var normalPerp = perpendicularVector(normalDir);
        var hs = size * 0.2;
        var nt1 = normalTip - hs * normalDir + hs * 0.3 * normalPerp;
        var nt2 = normalTip - hs * normalDir - hs * 0.3 * normalPerp;
        opFitSpline(context, id + "normalShaft", {
                "points" : [plane.origin, normalTip]
        });
        opFitSpline(context, id + "normalHead1", {
                "points" : [normalTip, nt1]
        });
        opFitSpline(context, id + "normalHead2", {
                "points" : [normalTip, nt2]
        });
        nameDebugBody(context, qCreatedBy(id, EntityType.BODY), color, label);
    }
    else
    {
        debug(context, plane, color);
    }
}
```

### Usage in a Feature

The feature adds a single boolean toggle and passes it as `wireMode` to every
debug call. The same call produces either a `debug()` overlay (for the user)
or a wire body (for the agent):

```featurescript
// Import the helpers
// (use the document/version/element path for the debug-wire-helpers Feature Studio)

annotation { "Feature Type Name" : "My Feature" }
export const myFeature = defineFeature(function(context is Context, id is Id,
        definition is map)
    precondition
    {
        // ... normal parameters ...

        annotation { "Name" : "Debug wire bodies",
                     "Default" : false }
        definition.debugWires is boolean;
    }
    {
        var wm = definition.debugWires;

        // ... compute geometry ...

        // Single call — dispatches to debug() or wire body based on toggle
        debugWireLine(context, id + "dbgNormalA", wm,
                frame.origin,
                frame.origin + 5 * millimeter * frame.normalA,
                DebugColor.RED, "normalA");

        debugWireLine(context, id + "dbgNormalB", wm,
                frame.origin,
                frame.origin + 5 * millimeter * frame.normalB,
                DebugColor.BLUE, "normalB");

        debugWirePoint(context, id + "dbgOrigin", wm,
                frame.origin, 0.5 * millimeter,
                DebugColor.ORANGE, "frameOrigin");

        debugWireSpline(context, id + "dbgProfile", wm,
                profilePoints,
                DebugColor.MAGENTA, "profile");

        // ... boolean operations ...
        // Wire bodies (if created) persist alongside the model.
        // They do not interfere with solid boolean operations.
    });
```

### Taking Screenshots with Wire Bodies

Use `onshape_screenshot` with `include_wires: true` to capture the debug wire
geometry:

```json
{
  "did": "...",
  "wvm": "w",
  "wvmid": "...",
  "eid": "...",
  "view": { "type": "preset", "name": "isometric" },
  "output_path": "/tmp/debug-view.png",
  "show_all_parts": true,
  "include_wires": true
}
```

### Agent Workflow

1. **Enable wire mode:** Update the feature definition via
   `updatePartStudioFeature` to set `debugWires = true`.
2. **Take screenshot:** Call `onshape_screenshot` with `include_wires: true`.
3. **Inspect:** The screenshot shows debug wire geometry alongside the model.
   Wire bodies are named `"debug-RED-normalA"`, `"debug-MAGENTA-profile"`,
   etc. Use `evalFeatureScript` or `getPartStudioBodyDetails` to list body
   names if needed.
4. **Disable wire mode:** Update the feature definition to set
   `debugWires = false` to restore normal model state (no wire bodies).

### Behavior Summary

| `wireMode` | What happens | Visible in browser | Visible in screenshot |
| ---------- | ------------ | ------------------ | --------------------- |
| `false` | Standard `debug()` / `addDebugLine` / etc. | Yes (edit dialog) | No |
| `true` | Wire body + named `"debug-{COLOR}-{label}"` | Yes (always) | Yes (`includeWires`) |

### Wire Mode Limitations

- **No visual color** — wire bodies render in a single default color in
  shaded views. The color intent is encoded in the body name
  (`"debug-RED-..."`) for programmatic identification.
- **No text labels in viewport** — body names are visible in the parts list
  but not overlaid on the 3D view.
- **Arrow/direction indicators are approximate** — the arrowhead is two
  diagonal lines, not a filled triangle.
- **Point markers use cross-shaped wire geometry** — point bodies may not
  render in shaded views, so a small 3D cross is used instead.
- **Performance** — creating many wire bodies (e.g., per-sample-point debug
  geometry on multi-edge chains) slows feature regeneration. Use sparingly.
- **Entity highlighting is edge-based** — `debugWireEntities` in wire mode
  extracts edge copies. Face highlighting becomes boundary-edge highlighting.
  Vertex highlighting is not supported in wire mode.

## Combining Both Approaches

The most effective workflow uses both approaches together:

1. **`evalFeatureScript` for exact data** — when you need specific numeric
   values (coordinates, normals, curvature, distances). Fast and precise.
2. **Wire mode for spatial context** — when you need to see how debug
   geometry relates to the model in 3D. Slower but gives visual intuition.
3. **Regular `debug()` for the user** — always active when `wireMode` is
   `false` (the default). The user sees colored overlays when editing
   features interactively.

## Pitfalls

1. **`includeWires` is off by default** — if you take a screenshot without
   `include_wires: true`, wire bodies are invisible. Always set this flag
   when using wire mode.

2. **`showAllParts` may be needed** — if wire body visibility settings in
   the Part Studio hide them, use `show_all_parts: true` in the screenshot
   call.

3. **Unit mismatch in `evalFeatureScript`** — forgetting to divide by units
   produces values with embedded unit metadata that is harder to interpret.
   Always divide lengths by `meter`, angles by `radian` or `degree`.

4. **Disable wire mode after inspection** — leaving `debugWires = true`
   adds wire bodies to the Part Studio that the user doesn't need. Always
   toggle it back to `false` after taking screenshots.

5. **`id` uniqueness in wire mode** — each `debugWire*` call in wire mode
   creates geometry under the given `id`. Follow the same id-uniqueness
   rules as any other FeatureScript operation. Use distinct id suffixes
   for each call (e.g., `id + "dbgNormalA"`, `id + "dbgNormalB"`).
