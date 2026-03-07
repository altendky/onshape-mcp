# Revolve

How to create revolve features via the Onshape Features API, and the workflow
for going from a sketch half-profile to a revolved solid body.

This document was derived from the
[crayon profile exercise](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/w/c25e3f6fa28d58a69e60d91d/e/7937799504a65744e79b713e),
where a half-profile sketch was revolved 360 degrees around a sketch edge to
produce a solid crayon shape.

## Sketch-to-Revolve Workflow

Creating a revolved solid from a sketch is a multi-step process. Each step
depends on outputs from the previous step. The workflow is analogous to the
[Extrude](extrude.md) sketch-to-solid workflow but adds axis discovery.

### Step 1: Create the sketch

Call `addPartStudioFeature` with a `BTMSketch-151` body (see the
[Sketch](sketch.md) insight for details). The sketch should contain a closed
half-profile suitable for revolution. The response includes:

- `feature.featureId` — needed to query sketch regions and edges
- `sourceMicroversion` — feed into subsequent calls

#### Half-profile design for revolve

A revolve half-profile is a closed loop of sketch entities where one edge lies
along the intended axis of revolution. All edges forming the profile boundary
**must** be non-construction (`isConstruction: false`) for a sketch region to
be produced. See [Sketch Pitfall #17](sketch.md#pitfalls).

For a typical revolve profile:

- One edge of the closed loop serves as both the revolve axis **and** a profile
  boundary edge (non-construction)
- The remaining edges define the cross-section shape
- The profile sits entirely on one side of the axis

### Step 2: Discover the sketch region deterministic ID

Same as for extrude — sketch regions are not directly addressable by entity ID.
Discover their deterministic IDs via `evalFeatureScript`:

```javascript
function(context is Context, queries is map) {
    return evaluateQuery(context, qCreatedBy(makeId("<featureId>"), EntityType.FACE));
}
```

Replace `<featureId>` with the sketch's `featureId`. The response contains
`transientId` values (e.g. `"JGC"`) that serve as `deterministicIds` for the
region query.

### Step 3: Discover the axis edge deterministic ID

The revolve axis requires a reference to a specific sketch edge. Unlike sketch
regions, sketch edges must also be discovered via `evalFeatureScript`:

```javascript
function(context is Context, queries is map) {
    var edges = evaluateQuery(context, qCreatedBy(makeId("<featureId>"), EntityType.EDGE));
    var results = [];
    for (var i = 0; i < size(edges); i += 1) {
        var endpoints = evEdgeTangentLines(context, {
            "edge" : edges[i], "parameters" : [0, 1]
        });
        results = append(results, {
            "index" : i,
            "query" : edges[i],
            "start" : endpoints[0].origin,
            "end" : endpoints[1].origin,
            "length" : evLength(context, { "entities" : edges[i] })
        });
    }
    return results;
}
```

This returns all sketch edges with their endpoints and transient IDs. Identify
the axis edge by matching its start/end coordinates and length to the expected
axis line.

**Important:** Sketch edge queries typically return **duplicate edges** — each
sketch line appears twice (once as the sketch entity edge and once as the face
boundary edge). Both have different transient IDs but identical geometry. Either
ID works for the revolve axis query, but the first occurrence (lower index) is
the sketch entity edge.

### Step 4: Create the revolve

Call `addPartStudioFeature` with the revolve feature, referencing the discovered
region ID via `BTMIndividualSketchRegionQuery-140` and the axis edge via
`BTMIndividualQuery-138`.

## Revolve Feature Structure

```json
{
  "btType": "BTMFeature-134",
  "featureType": "revolve",
  "name": "My Revolve",
  "parameters": [
    { "bodyType": "SOLID" },
    { "operationType": "NEW" },
    { "entities": "«sketch region query»" },
    { "axis": "«axis edge query»" },
    { "revolveType": "FULL" }
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

### `entities` — Sketch regions to revolve

Same query mechanism as extrude. Use `BTMIndividualSketchRegionQuery-140`:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "entities",
  "queries": [
    {
      "btType": "BTMIndividualSketchRegionQuery-140",
      "featureId": "FrslavMeMs6sheI_0",
      "deterministicIds": ["JGC"]
    }
  ]
}
```

### `axis` — Revolve axis

References a linear edge (typically a sketch line) to revolve around. Use
`BTMIndividualQuery-138` with the edge's deterministic ID from Step 3:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "axis",
  "queries": [
    {
      "btType": "BTMIndividualQuery-138",
      "deterministicIds": ["JFR"]
    }
  ]
}
```

The axis can be any linear edge: a sketch line, a construction line, a model
edge, or a default axis. The axis does **not** need to be a construction entity.

### `revolveType` — Revolution extent

| Value | Description |
| ----- | ----------- |
| `FULL` | Full 360-degree revolution (default for solid crayons, rings, etc.) |
| `ONE_DIRECTION` | Revolve by a specified angle in one direction |
| `SYMMETRIC` | Revolve by a specified angle symmetrically (half each direction) |
| `TWO_DIRECTIONS` | Revolve by different angles in each direction |

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "revolveType",
  "enumName": "RevolveType",
  "value": "FULL"
}
```

### `angle` — Revolution angle

Only used when `revolveType` is `ONE_DIRECTION`, `SYMMETRIC`, or
`TWO_DIRECTIONS`. Accepts unit expressions.

```json
{
  "btType": "BTMParameterQuantity-147",
  "parameterId": "angle",
  "expression": "180 deg",
  "isInteger": false
}
```

### Direction and second direction

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `oppositeDirection` | boolean | Flip revolve direction |
| `secondDirectionAngle` | quantity | Angle for second direction (when `revolveType` is `TWO_DIRECTIONS`) |

### Boolean scope

Same as [Extrude boolean scope](extrude.md#boolean-scope). When `operationType`
is `ADD`, `REMOVE`, or `INTERSECT`, use `defaultScope` and `booleanScope` to
control which bodies participate.

## Complete Example: Full Solid Revolve

This example revolves a crayon half-profile 360 degrees around the axis line
edge to produce a solid body:

```json
{
  "feature": {
    "btType": "BTMFeature-134",
    "featureType": "revolve",
    "name": "Revolve - Crayon",
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
        "value": "NEW"
      },
      {
        "btType": "BTMParameterQueryList-148",
        "parameterId": "entities",
        "queries": [
          {
            "btType": "BTMIndividualSketchRegionQuery-140",
            "featureId": "FrslavMeMs6sheI_0",
            "deterministicIds": ["JGC"]
          }
        ]
      },
      {
        "btType": "BTMParameterQueryList-148",
        "parameterId": "axis",
        "queries": [
          {
            "btType": "BTMIndividualQuery-138",
            "deterministicIds": ["JFR"]
          }
        ]
      },
      {
        "btType": "BTMParameterEnum-145",
        "parameterId": "revolveType",
        "enumName": "RevolveType",
        "value": "FULL"
      }
    ]
  },
  "libraryVersion": 2892,
  "serializationVersion": "1.2.16",
  "sourceMicroversion": "<microversion-from-previous-call>"
}
```

## Pitfalls

1. **Axis edge requires discovery** — Like sketch regions, the revolve axis
   edge's deterministic ID must be discovered via `evalFeatureScript` with
   `EntityType.EDGE`. You cannot use the sketch entity ID (e.g. `"axisLine"`)
   directly — that is a sketch-internal identifier, not a geometry query ID.

2. **Duplicate edges in sketch queries** — Querying
   `qCreatedBy(makeId("<featureId>"), EntityType.EDGE)` returns **twice** as
   many edges as sketch entities. Each sketch line appears as both a sketch
   entity edge and a face boundary edge. Match by endpoint coordinates and
   length to identify the correct edge. Either duplicate works for the axis
   query.

3. **Axis must be linear** — The axis query must resolve to a straight line
   (sketch line, model edge, or construction axis). Arcs, splines, and circles
   are not valid revolve axes.

4. **Profile must not cross the axis** — The sketch region being revolved must
   lie entirely on one side of the axis (or touch it). Profiles that cross the
   axis produce invalid geometry.

5. **Profile edge on the axis is valid** — Unlike some CAD systems, Onshape
   allows the revolve profile to include an edge that lies on the axis. This
   edge becomes a degenerate (zero-radius) surface and is handled correctly.
   This is the standard pattern for revolve half-profiles.

6. **`revolveType` enum name is `RevolveType`** — Not `RevolveTypeEnum` or
   `RevolutionType`. Using the wrong enum name causes silent failures.

7. **`angle` is ignored when `revolveType` is `FULL`** — Setting an angle with
   `FULL` revolve type has no effect. Use `ONE_DIRECTION` or `SYMMETRIC` for
   partial revolutions.

8. **Feed `sourceMicroversion` forward** — Same as extrude. Each
   `addPartStudioFeature` response includes a `sourceMicroversion`. Use it in
   subsequent calls to avoid microversion skew.
