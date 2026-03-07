# Sweep

How to create sweep features via the Onshape Features API, and the workflow
for going from a path sketch and profile sketch to a swept solid body.

This document was derived from the
[coffee mug exercise](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/w/c25e3f6fa28d58a69e60d91d/e/25cbc4078053af59e2007f6c),
where a circular profile was swept along an arc path to produce a mug handle,
then merged with the revolved mug body.

## Sketch-to-Sweep Workflow

Creating a swept solid is a multi-step process with more steps than extrude or
revolve, because the profile sketch requires a construction plane perpendicular
to the path.

### Step 1: Create the path sketch

Call `addPartStudioFeature` with a `BTMSketch-151` containing the sweep path
geometry (e.g. an arc, spline, or connected edges). The response includes:

- `feature.featureId` — needed to query path edges and vertices
- `sourceMicroversion` — feed into subsequent calls

### Step 2: Discover the path edge and start vertex

The sweep path edge and its start vertex must be discovered via
`evalFeatureScript`:

```javascript
function(context is Context, queries is map) {
    var edges = evaluateQuery(context, qCreatedBy(makeId("<featureId>"), EntityType.EDGE));
    var vertices = evaluateQuery(context, qCreatedBy(makeId("<featureId>"), EntityType.VERTEX));
    var vertexData = [];
    for (var i = 0; i < size(vertices); i += 1) {
        var pos = evVertexPoint(context, { "vertex" : vertices[i] });
        vertexData = append(vertexData, {
            "index" : i,
            "query" : vertices[i],
            "position" : pos
        });
    }
    return { "edges" : edges, "vertices" : vertexData };
}
```

You need both the edge's `transientId` (for the sweep `path` parameter) and the
start vertex's `transientId` (for creating the perpendicular profile plane in
Step 3).

### Step 3: Create a construction plane perpendicular to the path

The profile sketch must be on a plane perpendicular to the path at its start
point. Use a `cPlane` feature with `CURVE_POINT` mode (see the
[Construction Plane](cplane.md) insight):

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
          { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<path-edge-id>"] },
          { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<start-vertex-id>"] }
        ]
      }
    ]
  }
}
```

Discover the cPlane's face ID via `evalFeatureScript` with
`qCreatedBy(makeId("<cPlaneFeatureId>"), EntityType.FACE)`.

### Step 4: Create the profile sketch on the perpendicular plane

Create a sketch on the cPlane face. **Critically**, constrain the profile center
to the path start vertex using an external COINCIDENT constraint:

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "COINCIDENT",
  "entityId": "cAlign",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "profileCircle.center"
    },
    {
      "btType": "BTMParameterQueryList-148",
      "parameterId": "externalSecond",
      "queries": [
        { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<start-vertex-id>"] }
      ]
    }
  ]
}
```

Without this constraint, the profile center may not coincide with the path
endpoint, causing the swept body to be offset from the path.

### Step 5: Discover the profile region deterministic ID

Same as for extrude — discover the sketch region via `evalFeatureScript`:

```javascript
function(context is Context, queries is map) {
    return evaluateQuery(context, qCreatedBy(makeId("<profileSketchId>"), EntityType.FACE));
}
```

### Step 6: Create the sweep

Call `addPartStudioFeature` with the sweep feature, referencing the profile
region via `BTMIndividualSketchRegionQuery-140` and the path edge via
`BTMIndividualQuery-138`.

## Sweep Feature Structure

```json
{
  "btType": "BTMFeature-134",
  "featureType": "sweep",
  "name": "My Sweep",
  "parameters": [
    { "bodyType": "SOLID" },
    { "operationType": "NEW" },
    { "profiles": "«profile region query»" },
    { "path": "«path edge query»" }
  ]
}
```

## Parameters

### `bodyType` — Creation type

Same as [Extrude `bodyType`](extrude.md#bodytype--creation-type).

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "bodyType",
  "enumName": "ExtendedToolBodyType",
  "value": "SOLID"
}
```

### `operationType` — Boolean operation

Same as [Extrude `operationType`](extrude.md#operationtype--boolean-operation).

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "operationType",
  "enumName": "NewBodyOperationType",
  "value": "NEW"
}
```

### `profiles` — Sketch regions to sweep

References the sketch region(s) to sweep. Same query mechanism as extrude:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "profiles",
  "queries": [
    {
      "btType": "BTMIndividualSketchRegionQuery-140",
      "featureId": "<profile-sketch-feature-id>",
      "deterministicIds": ["<region-id>"]
    }
  ]
}
```

The parameter ID changes depending on `bodyType`:

| `bodyType` | Parameter ID | Accepts |
| ---------- | ------------ | ------- |
| `SOLID` | `profiles` | Faces and sketch regions |
| `SURFACE` | `surfaceProfiles` | Edges and sketch curves |
| `THIN` | `wallShape` | Faces, edges, or wire bodies |

### `path` — Sweep path

References the edge(s) to sweep along:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "path",
  "queries": [
    {
      "btType": "BTMIndividualQuery-138",
      "deterministicIds": ["<path-edge-id>"]
    }
  ]
}
```

The path can be a single edge (arc, line, spline) or a chain of connected edges.

### `profileControl` — Profile orientation

| Value | Description |
| ----- | ----------- |
| `NONE` | Default — profile follows the Frenet frame of the path |
| `KEEP_ORIENTATION` | Maintain the initial profile orientation along the path |
| `LOCK_FACES` | Lock specific faces to maintain their orientation |
| `LOCK_DIRECTION` | Lock the profile to a specific direction |

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "profileControl",
  "enumName": "ProfileControlMode",
  "value": "NONE"
}
```

### Twist and scale

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `hasTwist` | boolean | Enable twist along the path |
| `twistType` | `SweepTwistType` enum | `TURNS`, `ANGLE`, or `PITCH` |
| `turns` | quantity (real) | Number of revolutions (when `twistType` is `TURNS`) |
| `angle` | quantity (angle) | Rotation angle (when `twistType` is `ANGLE`) |
| `pitch` | quantity (length) | Pitch length (when `twistType` is `PITCH`) |
| `hasScale` | boolean | Enable scaling along the path |
| `scaleFactor` | quantity (real) | Scale factor at the end of the path |

### Boolean scope

Same as [Extrude boolean scope](extrude.md#boolean-scope). When `operationType`
is `ADD`, `REMOVE`, or `INTERSECT`, use `defaultScope` and `booleanScope` to
control which bodies participate.

## Complete Example: Solid Sweep (Mug Handle)

This example sweeps a 12mm circular profile along an arc path and merges the
result with an existing mug body:

```json
{
  "feature": {
    "btType": "BTMFeature-134",
    "featureType": "sweep",
    "name": "Sweep - Handle",
    "parameters": [
      {
        "btType": "BTMParameterEnum-145",
        "parameterId": "bodyType",
        "enumName": "ExtendedToolBodyType",
        "value": "SOLID"
      },
      {
        "btType": "BTMParameterEnum-145",
        "parameterId": "operationType",
        "enumName": "NewBodyOperationType",
        "value": "ADD"
      },
      {
        "btType": "BTMParameterQueryList-148",
        "parameterId": "profiles",
        "queries": [
          {
            "btType": "BTMIndividualSketchRegionQuery-140",
            "featureId": "<profile-sketch-feature-id>",
            "deterministicIds": ["<profile-region-id>"]
          }
        ]
      },
      {
        "btType": "BTMParameterQueryList-148",
        "parameterId": "path",
        "queries": [
          {
            "btType": "BTMIndividualQuery-138",
            "deterministicIds": ["<path-edge-id>"]
          }
        ]
      },
      {
        "btType": "BTMParameterBoolean-144",
        "parameterId": "defaultScope",
        "value": true
      }
    ]
  },
  "serializationVersion": "1.2.16",
  "sourceMicroversion": "<microversion-from-previous-call>"
}
```

## Pitfalls

1. **Profile must be on a plane perpendicular to the path** — The profile sketch
   must be on a plane whose normal is approximately along the path tangent at
   the start point. A profile and path on the same sketch plane will fail with
   an ERROR status because the profile face normal is perpendicular to the path
   tangent rather than along it. Use a `cPlane` with `CURVE_POINT` mode to
   create the correct plane (see [Construction Plane](cplane.md)).

2. **Profile center must be aligned to the path endpoint** — Always add an
   external COINCIDENT constraint from the profile center (e.g.
   `"circle.center"`) to the path start vertex. Without this, the profile may
   be offset from the path, producing a swept body that doesn't follow the
   intended path. The sketch origin on a `cPlane` does not automatically
   coincide with the vertex used to create the plane.

3. **Sweep is a 6-step workflow** — Unlike extrude (3 steps) or revolve
   (4 steps), sweep requires: path sketch → discover edge + vertex → create
   cPlane → profile sketch on cPlane → discover profile region → sweep. Missing
   any intermediate discovery step will cause the sweep to fail.

4. **`profiles` parameter ID varies by `bodyType`** — For `SOLID` use
   `profiles`, for `SURFACE` use `surfaceProfiles`, for `THIN` use `wallShape`.
   Using the wrong parameter ID causes the profile selection to be silently
   ignored.

5. **Feed `sourceMicroversion` forward** — Same as extrude and revolve. Each
   `addPartStudioFeature` response includes a `sourceMicroversion`. Use it in
   subsequent calls to avoid microversion skew.

6. **Profile placement determines sweep extent** — The profile can be placed
   at either end of the path. If the profile is placed in the middle of the
   path, the sweep extends in both directions. The workflow above uses the
   path start vertex for the construction plane as a convention, but placing
   the profile at the other end is equally valid.
