# Extrude

How to create extrude features via the Onshape Features API, and the workflow
for going from a sketch to a solid body.

## Sketch-to-Extrude Workflow

Creating a solid from a sketch is a multi-step process. Each step depends on
outputs from the previous step.

### Step 1: Create the sketch

Call `addPartStudioFeature` with a `BTMSketch-151` body (see the
[Sketch](sketch.md) insight for details). The response includes:

- `feature.featureId` — needed to query sketch regions
- `sourceMicroversion` — feed into subsequent calls

### Step 2: Discover the sketch region deterministic ID

Sketch regions (the closed faces formed by sketch curves) are not directly
addressable by entity ID. You must discover their deterministic IDs via
`evalFeatureScript`:

```javascript
function(context is Context, queries is map) {
    return evaluateQuery(context, qCreatedBy(makeId("<featureId>"), EntityType.FACE));
}
```

Replace `<featureId>` with the sketch's `featureId` from Step 1. The response
contains `transientId` values (e.g. `"JGC"`) that serve as `deterministicIds`
for the region query.

A simple rectangle sketch produces one region. Complex sketches with multiple
closed loops produce multiple regions — inspect the array to pick the right one.

### Step 3: Create the extrude

Call `addPartStudioFeature` with the extrude feature, referencing the discovered
region ID via `BTMIndividualSketchRegionQuery-140`.

## Extrude Feature Structure

```json
{
  "btType": "BTMFeature-134",
  "featureType": "extrude",
  "name": "My Extrude",
  "parameters": [
    { "bodyType": "SOLID" },
    { "operationType": "NEW" },
    { "entities": "«sketch region query»" },
    { "endBound": "BLIND" },
    { "depth": "«expression»" }
  ]
}
```

## Parameters

### `bodyType` — Creation type

| Value | Description |
| ----- | ----------- |
| `SOLID` | Solid body (default) |
| `SURFACE` | Surface body |
| `THIN` | Thin-wall body |

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "bodyType",
  "enumName": "ExtendedToolBodyType",
  "value": "SOLID"
}
```

### `operationType` — Boolean operation

| Value | Description |
| ----- | ----------- |
| `NEW` | Create a new body (default) |
| `ADD` | Add to existing body |
| `REMOVE` | Remove from existing body |
| `INTERSECT` | Intersect with existing body |

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "operationType",
  "enumName": "NewBodyOperationType",
  "value": "NEW"
}
```

### `entities` — Sketch regions to extrude

References the sketch region(s) to extrude. Use `BTMIndividualSketchRegionQuery-140`,
which extends `BTMIndividualQuery-138` with a `featureId` field that ties the
query to a specific sketch.

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "entities",
  "queries": [
    {
      "btType": "BTMIndividualSketchRegionQuery-140",
      "featureId": "FyTUh237nlUGxBL_0",
      "deterministicIds": ["JGC"]
    }
  ]
}
```

- `featureId` — the sketch feature's ID
- `deterministicIds` — the region's transient ID from `evalFeatureScript`

For surface extrudes, use `surfaceEntities` (parameterId) instead, referencing
sketch edges rather than faces.

### `endBound` — End type

| Value | Description |
| ----- | ----------- |
| `BLIND` | Fixed depth (default) |
| `UP_TO_NEXT` | Extend to next solid face |
| `UP_TO_SURFACE` | Extend to a selected face |
| `UP_TO_BODY` | Extend to a selected body |
| `UP_TO_VERTEX` | Extend to a selected vertex |
| `THROUGH_ALL` | Extend through all geometry |

```json
{
  "btType": "BTMParameterEnum-145",
  "parameterId": "endBound",
  "enumName": "BoundingType",
  "value": "BLIND"
}
```

### `depth` — Extrusion depth

Only used when `endBound` is `BLIND`. Accepts unit expressions.

```json
{
  "btType": "BTMParameterQuantity-147",
  "parameterId": "depth",
  "expression": "0.25 in",
  "isInteger": false
}
```

### Direction and symmetry

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `oppositeDirection` | boolean | Flip extrude direction |
| `symmetric` | boolean | Extrude equally in both directions |
| `midplane` | boolean | Center on sketch plane |
| `hasSecondDirection` | boolean | Enable asymmetric second direction |
| `secondDirectionBound` | `BoundingType` enum | Second direction end type |
| `secondDirectionDepth` | quantity | Second direction depth |
| `hasDraft` | boolean | Enable draft angle |
| `draftAngle` | quantity | Draft angle value |

### Boolean scope

When `operationType` is `ADD`, `REMOVE`, or `INTERSECT`:

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `defaultScope` | boolean | `true` = merge with all intersecting bodies |
| `booleanScope` | query list | Specific bodies to merge with (when `defaultScope` is `false`) |

## Complete Example: Blind Solid Extrude

```json
{
  "feature": {
    "btType": "BTMFeature-134",
    "featureType": "extrude",
    "name": "Extrude 1",
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
            "featureId": "<sketch-feature-id>",
            "deterministicIds": ["<region-id>"]
          }
        ]
      },
      {
        "btType": "BTMParameterEnum-145",
        "parameterId": "endBound",
        "enumName": "BoundingType",
        "value": "BLIND"
      },
      {
        "btType": "BTMParameterQuantity-147",
        "parameterId": "depth",
        "expression": "0.25 in",
        "isInteger": false
      }
    ]
  },
  "libraryVersion": 2892,
  "serializationVersion": "1.2.16",
  "sourceMicroversion": "<microversion-from-previous-call>"
}
```

## Pitfalls

1. **Sketch regions require discovery** — You cannot reference a sketch region
   by entity ID. You must call `evalFeatureScript` with
   `qCreatedBy(makeId("<featureId>"), EntityType.FACE)` to obtain the
   deterministic ID.

2. **Use `BTMIndividualSketchRegionQuery-140`**, not plain
   `BTMIndividualQuery-138` — The sketch region query subtype includes a
   `featureId` field that ties the region to its sketch. Without it, Onshape
   cannot resolve the region.

3. **Feed `sourceMicroversion` forward** — Each `addPartStudioFeature` response
   includes a `sourceMicroversion`. Use it in subsequent calls. Stale
   microversions may cause `rejectMicroversionSkew` failures or silent geometry
   mismatches.

4. **`depth` is ignored unless `endBound` is `BLIND`** — Setting a depth with
   `THROUGH_ALL` or `UP_TO_SURFACE` has no effect.

5. **Enum names are specific** — `bodyType` uses `ExtendedToolBodyType` (not
   `BodyType`), `operationType` uses `NewBodyOperationType` (not
   `OperationType`), and `endBound` uses `BoundingType`. Using wrong enum names
   causes silent failures.

6. **Surface extrudes use `surfaceEntities`**, not `entities` — The parameter ID
   changes depending on `bodyType`. For `SOLID` use `entities` (sketch faces);
   for `SURFACE` use `surfaceEntities` (sketch edges).
