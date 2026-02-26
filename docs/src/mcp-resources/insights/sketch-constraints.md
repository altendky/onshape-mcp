# Sketch Constraints

How to correctly define sketch constraints via the Onshape Features API.

## Essential Pattern

Constraints reference their target sketch entities through **`BTMParameterString-149`
entries in the `parameters` array**, not through a top-level `entityIds` field.

The `entityId` field (singular) on `BTMSketchConstraint-2` is the constraint's **own
identity** (inherited from `BTMSketchEntity-3`), not a reference to target entities.

## Inheritance Chain

```text
BTMNode-19                     -> { nodeId }
  BTMSketchEntity-3            -> { entityId, parameters[], namespace, name, index }
    BTMSketchConstraint-2      -> { constraintType, drivenDimension, helpParameters[], ... }
```

## Entity Reference Mechanism

Target entities are specified via `BTMParameterString-149` parameters:

| `parameterId` | Description |
| -------------- | ----------- |
| `localFirst` | Entity ID of the first constrained entity |
| `localSecond` | Entity ID of the second constrained entity (for 2-entity constraints) |
| `localThird` | Entity ID of the third constrained entity (for mirror, etc.) |

Dimensional values are specified via `BTMParameterQuantity-147` in the same
`parameters` array.

## Constraint Types (`GBTConstraintType`)

Common types used for basic sketching:

| Type | Entities | Notes |
| ---- | -------- | ----- |
| `HORIZONTAL` | 1 (line) | Makes a line horizontal |
| `VERTICAL` | 1 (line) | Makes a line vertical |
| `COINCIDENT` | 2 (points or point+line) | Makes entities coincident |
| `EQUAL` | 2 (lines) | Makes lines equal length |
| `LENGTH` | 1 (line) + dimension | Sets line length |
| `DISTANCE` | 2 (entities) + dimension | Sets distance between entities |
| `FIX` | 1 (any) | Pins entity in place |
| `PERPENDICULAR` | 2 (lines) | Makes lines perpendicular |
| `PARALLEL` | 2 (lines) | Makes lines parallel |
| `TANGENT` | 2 (curves) | Makes curves tangent |
| `CONCENTRIC` | 2 (arcs/circles) | Makes arcs share center |
| `MIDPOINT` | 2 (point + line) | Places point at midpoint |

Full enum also includes: `NONE`, `MIRROR`, `NORMAL`, `PROJECTED`, `OFFSET`,
`CIRCULAR_PATTERN`, `PIERCE`, `LINEAR_PATTERN`, `MAJOR_DIAMETER`,
`MINOR_DIAMETER`, `QUADRANT`, `DIAMETER`, `SILHOUETTED`,
`CENTERLINE_DIMENSION`, `INTERSECTED`, `RHO`, `EQUAL_CURVATURE`,
`BEZIER_DEGREE`, `FREEZE`, `RADIUS`, `ANGLE`, `UNKNOWN`.

## Example Constraint JSON

### HORIZONTAL (single entity)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "HORIZONTAL",
  "nodeId": "some-unique-node-id",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1"
    }
  ]
}
```

### COINCIDENT (two entities)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "COINCIDENT",
  "nodeId": "some-unique-node-id",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1.end"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localSecond",
      "value": "line2.start"
    }
  ]
}
```

### EQUAL (two entities)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "EQUAL",
  "nodeId": "some-unique-node-id",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localSecond",
      "value": "line2"
    }
  ]
}
```

### LENGTH (dimension on single entity)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "LENGTH",
  "nodeId": "some-unique-node-id",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1"
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "length",
      "expression": "1 cm",
      "isInteger": false
    }
  ]
}
```

## Pitfalls

1. **Do not use `entityIds` (plural)** — This field does not exist on
   `BTMSketchConstraint-2`. The API silently drops it, resulting in constraints
   with `entityId: ""` that produce warnings: *"Some constraints are not
   applicable to the current external references and have not been solved."*

2. **Use `LENGTH`, not `DISTANCE`**, for a single-line dimension — `LENGTH`
   constrains one line's length. `DISTANCE` sets the distance between two
   entities.

3. **Always include `localFirst`/`localSecond`** — Without these
   `BTMParameterString-149` entries in the `parameters` array, the constraint
   has no entity references and will be ignored by the solver.

## Sketch Constraint Degree-of-Freedom Analysis

For a closed square (4 lines, each with 2 endpoints):

- 4 lines = 16 DOF (4 per line: start x/y, end x/y)
- 4 coincident constraints = -8 DOF (each removes 2)
- 2 horizontal constraints = -2 DOF (each removes 1)
- 2 vertical constraints = -2 DOF (each removes 1)
- 1 equal constraint = -1 DOF
- 1 length dimension = -1 DOF
- Remaining: 2 DOF (x, y translation of the square)

To fully constrain, add a `FIX` or `COINCIDENT` to pin a point to the origin,
or add two more dimensions.

## Discovering New Constraint Formats

For constraint types not yet documented here, create the constraint manually in
the Onshape UI, then call `getPartStudioFeatures` to inspect the resulting JSON.
This is the most reliable way to determine the correct `parameters` structure.
