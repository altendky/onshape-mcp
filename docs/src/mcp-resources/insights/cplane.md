# Construction Plane

How to create construction planes (`cPlane` features) via the Onshape Features
API for use as sketch planes in downstream features.

This document was derived from the
[coffee mug exercise](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/w/c25e3f6fa28d58a69e60d91d/e/25cbc4078053af59e2007f6c),
where a `CURVE_POINT` construction plane was used to create a sketch plane
perpendicular to a sweep path at its start vertex.

## When to Use Construction Planes

Construction planes are needed when a sketch must be placed on a plane that does
not already exist as a default plane (Front, Top, Right) or a planar face on an
existing body. Common use cases:

- **Sweep profiles** — a plane perpendicular to the sweep path at a vertex
  (see [Sweep](sweep.md))
- **Offset sketches** — a plane parallel to an existing plane at a specified
  distance
- **Angled sketches** — a plane rotated about a line from an existing plane

## Feature Structure

```json
{
  "btType": "BTMFeature-134",
  "featureType": "cPlane",
  "name": "My Plane",
  "parameters": [
    { "cplaneType": "CURVE_POINT" },
    { "entities": "«query list»" }
  ]
}
```

## Parameters

### `cplaneType` — Creation mode

| Value | Description | `entities` expects |
| ----- | ----------- | ------------------ |
| `OFFSET` | Parallel to a plane at a distance | 1 plane |
| `PLANE_POINT` | Through a plane's normal at a point | 1 plane + 1 point |
| `LINE_ANGLE` | Rotated about a line from a plane | 1 plane + 1 line |
| `LINE_POINT` | Through a point, normal to a line | 1 point + 1 line |
| `THREE_POINT` | Through three points | 3 points |
| `MID_PLANE` | Midway between two planes/faces | 2 planes or faces |
| `CURVE_POINT` | Perpendicular to a curve at a point | 1 edge + 1 vertex |
| `TANGENT_PLANE` | Tangent to a curved face | 1 curved face |

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "cplaneType",
  "enumName": "CPlaneType",
  "value": "CURVE_POINT"
}
```

### `entities` — Geometry references

All geometry references go into a single `entities` query list, regardless of
the creation mode. For `CURVE_POINT`, include both the edge and the vertex:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "entities",
  "queries": [
    { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<edge-id>"] },
    { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<vertex-id>"] }
  ]
}
```

For `OFFSET`, include only the plane:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "entities",
  "queries": [
    { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<plane-face-id>"] }
  ]
}
```

### `offset` — Offset distance

Used with `OFFSET` mode. Accepts unit expressions.

```json
{
  "btType": "BTMParameterQuantity-147",
  "parameterId": "offset",
  "expression": "25 mm",
  "isInteger": false
}
```

### `angle` — Rotation angle

Used with `LINE_ANGLE` mode. Accepts unit expressions.

```json
{
  "btType": "BTMParameterQuantity-147",
  "parameterId": "angle",
  "expression": "45 deg",
  "isInteger": false
}
```

### Direction and alignment

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `oppositeDirection` | boolean | Flip offset or angle direction |
| `flipAlignment` | boolean | Flip the plane's in-plane orientation |
| `flipNormal` | boolean | Flip the plane's normal direction |

### Visual extent

The `width` and `height` parameters control the visual size of the plane in the
UI. They have no effect on geometry but a plane that is much larger than the
model is visually distracting.

```json
{
  "btType": "BTMParameterQuantity-147",
  "parameterId": "width",
  "expression": "100 mm",
  "isInteger": false
}
```

| parameterId | Type | Default | Description |
| ----------- | ---- | ------- | ----------- |
| `width` | quantity (length) | 0.15 m | Horizontal visual extent |
| `height` | quantity (length) | 0.15 m | Vertical visual extent |

**Sizing heuristic:** Query `getPartStudioBoundingBoxes` and set `width` and
`height` to approximately the largest bounding box dimension. After sketching on
the plane, optionally refine to 2–3x the largest sketch entity dimension using
`updatePartStudioFeature`.

## Referencing a cPlane as a Sketch Plane

After creating a cPlane, discover its face ID:

```javascript
function(context is Context, queries is map) {
    return evaluateQuery(context, qCreatedBy(makeId("<cPlaneFeatureId>"), EntityType.FACE));
}
```

The response contains a `transientId` value that serves as the `deterministicIds`
entry for the sketch's `sketchPlane` parameter:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "sketchPlane",
  "queries": [
    {
      "btType": "BTMIndividualQuery-138",
      "deterministicIds": ["<cplane-face-id>"]
    }
  ]
}
```

## Complete Example: Perpendicular to Curve at Vertex

This example creates a plane perpendicular to a sweep path arc at its start
vertex, suitable for placing a sweep profile sketch:

```json
{
  "feature": {
    "btType": "BTMFeature-134",
    "featureType": "cPlane",
    "name": "Profile Plane",
    "parameters": [
      {
        "btType": "BTMParameterEnum-145",
        "parameterId": "cplaneType",
        "enumName": "CPlaneType",
        "value": "CURVE_POINT"
      },
      {
        "btType": "BTMParameterQueryList-148",
        "parameterId": "entities",
        "queries": [
          {
            "btType": "BTMIndividualQuery-138",
            "deterministicIds": ["<path-edge-id>"]
          },
          {
            "btType": "BTMIndividualQuery-138",
            "deterministicIds": ["<start-vertex-id>"]
          }
        ]
      },
      {
        "btType": "BTMParameterQuantity-147",
        "parameterId": "width",
        "expression": "95 mm",
        "isInteger": false
      },
      {
        "btType": "BTMParameterQuantity-147",
        "parameterId": "height",
        "expression": "95 mm",
        "isInteger": false
      }
    ]
  },
  "serializationVersion": "1.2.16",
  "sourceMicroversion": "<microversion-from-previous-call>"
}
```

## Pitfalls

1. **All references go in one `entities` list** — Unlike features that have
   separate parameters for different inputs (e.g. revolve has `entities` and
   `axis`), the cPlane uses a single `entities` parameter for all geometry
   references. For `CURVE_POINT`, include both the edge and the vertex in the
   same query list.

2. **Vertex discovery required for `CURVE_POINT`** — The vertex at the desired
   point on the curve must be discovered via `evalFeatureScript` with
   `EntityType.VERTEX`. Sketch arc/line endpoints are vertices.

3. **Default visual extent is 150mm** — The `width` and `height` default to
   0.15 m (150 mm), which dwarfs small models. Size the plane to match the model
   using the bounding box heuristic described above.

4. **cPlane face ID must be discovered** — Like sketch regions, the cPlane's
   face is not directly addressable. Call `evalFeatureScript` with
   `qCreatedBy(makeId("<featureId>"), EntityType.FACE)` to obtain the
   deterministic ID for use as a sketch plane.

5. **Enum name is `CPlaneType`** — Not `PlaneType` or `ConstructionPlaneType`.
   Using the wrong enum name causes silent failures.
