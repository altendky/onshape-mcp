# Sketches

How to create and structure sketch features via the Onshape Features API.

This document was derived from analysis of the
[sketch reference V2](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/v/98e9ea3fc5ac9c64559488b9/e/e138994dacce6d9757e6fc2b)
Part Studio, which contains 8 hand-crafted reference sketches covering lines,
circles, arcs, construction geometry, complex profiles, splines, ellipses,
patterns, offset planes, points, and slots.

## Sketch Feature Structure

A sketch is a `BTMSketch-151` (extends `BTMFeature-134`) with three key arrays:

```json
{
  "btType": "BTMSketch-151",
  "featureType": "newSketch",
  "name": "My Sketch",
  "parameters": [ /* sketch-level params: sketchPlane, disableImprinting */ ],
  "entities": [ /* BTMSketchGeomEntity-5 subtypes */ ],
  "constraints": [ /* BTMSketchConstraint-2 entries */ ],
  "subFeatures": [ /* e.g. mate connector for offset plane */ ]
}
```

## Sketch Planes

### Default Planes

Reference default planes via `BTMIndividualQuery-138` with `deterministicIds`.
The IDs are **per-element** and must be discovered via `evalFeatureScript`:

```javascript
// Call evalFeatureScript with this script to get plane IDs
function(context is Context, queries is map) {
    return {
        "front" : evaluateQuery(context, qCreatedBy(makeId("Front"), EntityType.FACE)),
        "top"   : evaluateQuery(context, qCreatedBy(makeId("Top"), EntityType.FACE)),
        "right" : evaluateQuery(context, qCreatedBy(makeId("Right"), EntityType.FACE))
    };
}
```

The response contains `transientId` values (e.g. `"JCC"`, `"JDC"`, `"JEC"`) that
serve as the `deterministicIds` for plane queries.

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "sketchPlane",
  "queries": [
    {
      "btType": "BTMIndividualQuery-138",
      "deterministicIds": ["JCC"]
    }
  ]
}
```

The origin point has deterministic ID `"IB"` and is referenced via
`externalSecond` parameters in COINCIDENT constraints.

### Offset Planes

Offset planes are encoded as a **mate connector subFeature** inside the sketch,
not as a separate feature. The sketch's `sketchPlane` query uses
`BTMIndividualCreatedByQuery-137` (not `BTMIndividualQuery-138`):

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "sketchPlane",
  "queries": [
    {
      "btType": "BTMIndividualCreatedByQuery-137",
      "bodyType": "MATE_CONNECTOR",
      "entityType": "BODY",
      "featureId": "FFWjHxvYTVB7W8p"
    }
  ]
}
```

The referenced mate connector lives in `subFeatures`:

```json
{
  "btType": "BTMFeature-134",
  "featureId": "FFWjHxvYTVB7W8p",
  "featureType": "mateConnector",
  "name": "Mate connector",
  "parameters": [
    { "parameterId": "originType", "value": "ON_ENTITY" },
    { "parameterId": "originQuery", "...": "query referencing Origin point" },
    { "parameterId": "realign", "value": true },
    { "parameterId": "primaryAxisQuery", "...": "query referencing Front plane" },
    { "parameterId": "transform", "value": true },
    { "parameterId": "translationZ", "expression": "1 in" },
    { "parameterId": "isForSubFeature", "value": true }
  ]
}
```

The offset distance is `translationZ`. The `primaryAxisQuery` sets which plane
the offset is relative to.

## Entity Types

### Inheritance Hierarchy

```text
BTMSketchGeomEntity-5         -> { isConstruction, entityId }
  BTMSketchCurve-4            -> { centerId, geometry }        (full closed curves)
    BTMSketchCurveSegment-155 -> { startPointId, endPointId,   (open/partial curves)
                                   startParam, endParam }
  BTMSketchPoint-158          -> { x, y, isUserPoint }         (points)
```

### Full Circles (`BTMSketchCurve-4`)

Full closed curves use `BTMSketchCurve-4` directly. They have **no** start/end
points or parameters.

```json
{
  "btType": "BTMSketchCurve-4",
  "entityId": "circle1",
  "centerId": "circle1.center",
  "isConstruction": false,
  "geometry": {
    "btType": "BTCurveGeometryCircle-115",
    "xCenter": 0.0,
    "yCenter": 0.0,
    "radius": 0.0127,
    "clockwise": false,
    "xDir": 1.0,
    "yDir": 0.0
  }
}
```

Dimension full circles with `DIAMETER` (not `RADIUS` or `LENGTH`):

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "DIAMETER",
  "parameters": [
    { "btType": "BTMParameterString-149", "parameterId": "localFirst", "value": "circle1" },
    { "btType": "BTMParameterQuantity-147", "parameterId": "length", "expression": "1 in" }
  ]
}
```

### Lines (`BTMSketchCurveSegment-155`)

```json
{
  "btType": "BTMSketchCurveSegment-155",
  "entityId": "line1",
  "startPointId": "line1.start",
  "endPointId": "line1.end",
  "startParam": -0.0254,
  "endParam": 0.0254,
  "isConstruction": false,
  "geometry": {
    "btType": "BTCurveGeometryLine-117",
    "pntX": 0.0254,
    "pntY": 0.0,
    "dirX": 1.0,
    "dirY": 0.0
  }
}
```

**Line parameterization:** `(pntX, pntY)` is **not** the start point. It is the
point at parameter `t = 0`. Start and end points are:

- start = `(pntX + startParam * dirX, pntY + startParam * dirY)`
- end = `(pntX + endParam * dirX, pntY + endParam * dirY)`

Typically `pntX/pntY` is the midpoint and `startParam = -endParam`.

### Arcs (`BTMSketchCurveSegment-155` with circle geometry)

An arc is a `BTMSketchCurveSegment-155` whose geometry is `BTCurveGeometryCircle-115`.
It differs from a full circle by having start/end params and points:

```json
{
  "btType": "BTMSketchCurveSegment-155",
  "entityId": "arc1",
  "centerId": "arc1.center",
  "startPointId": "arc1.start",
  "endPointId": "arc1.end",
  "startParam": -4.989,
  "endParam": -2.969,
  "isConstruction": false,
  "geometry": {
    "btType": "BTCurveGeometryCircle-115",
    "xCenter": -0.039,
    "yCenter": 0.039,
    "radius": 0.025,
    "clockwise": false,
    "xDir": 1.0,
    "yDir": 0.0
  }
}
```

The `startParam`/`endParam` are angles in radians on the underlying circle.

### Ellipses (`BTMSketchCurve-4`)

Full ellipses use `BTMSketchCurve-4` with `BTCurveGeometryEllipse-1189`:

```json
{
  "btType": "BTMSketchCurve-4",
  "entityId": "ellipse1",
  "centerId": "ellipse1.center",
  "isConstruction": false,
  "geometry": {
    "btType": "BTCurveGeometryEllipse-1189",
    "xCenter": 0.0,
    "yCenter": 0.0,
    "radius": 0.0508,
    "minorRadius": 0.0127,
    "clockwise": false,
    "xDir": 0.65,
    "yDir": 0.76
  }
}
```

- `radius` = major semi-axis length (meters)
- `minorRadius` = minor semi-axis length (meters)
- `xDir/yDir` = direction of the major axis

Dimension with `MAJOR_DIAMETER` and `MINOR_DIAMETER`:

```json
{ "constraintType": "MAJOR_DIAMETER", "parameters": [
    { "parameterId": "localFirst", "value": "ellipse1" },
    { "parameterId": "length", "expression": "4 in" }
]}
```

### Interpolated Splines (`BTMSketchCurveSegment-155`)

```json
{
  "btType": "BTMSketchCurveSegment-155",
  "entityId": "spline1",
  "startPointId": "spline1.start",
  "endPointId": "spline1.end",
  "startParam": 0.0,
  "endParam": 1.0,
  "internalIds": [
    "spline1.0.internal", "spline1.1.internal",
    "spline1.2.internal", "spline1.3.internal",
    "spline1.4.internal",
    "spline1.startHandle", "spline1.endHandle"
  ],
  "geometry": {
    "btType": "BTCurveGeometryInterpolatedSpline-116",
    "isPeriodic": false,
    "interpolationPoints": [0.0, 0.0, 0.01, 0.02, 0.03, 0.01, 0.04, 0.0],
    "startDerivativeX": 0.43,
    "startDerivativeY": 0.12,
    "endDerivativeX": -0.34,
    "endDerivativeY": 0.10,
    "startHandleX": 0.005,
    "startHandleY": 0.01,
    "endHandleX": 0.035,
    "endHandleY": -0.005
  }
}
```

- `interpolationPoints` is a **flat** array of `[x0, y0, x1, y1, ...]` pairs
- `internalIds` has one `.N.internal` per interpolation point, plus `.startHandle`
  and `.endHandle`
- Entity-level `parameters` include `splinePointCount` and per-point handle flags

### Points (`BTMSketchPoint-158`)

```json
{
  "btType": "BTMSketchPoint-158",
  "entityId": "point1",
  "isUserPoint": true,
  "isConstruction": false,
  "x": -0.028,
  "y": 0.012
}
```

Points have no `geometry` sub-object. Coordinates are directly on the entity.

### Construction Geometry

Any entity can be marked as construction by setting `isConstruction: true`.
Construction entities are used as references and do not contribute to profiles.

## Entity ID Conventions

All coordinates are in **meters** (SI base unit).

### Suffix Patterns

| Suffix | Meaning |
| ------ | ------- |
| `.start` | Start point of a curve segment |
| `.end` | End point of a curve segment |
| `.center` | Center point of a circle/arc/ellipse |
| `.N.internal` | Nth interpolation point of a spline |
| `.startHandle` | Spline start tangent handle |
| `.endHandle` | Spline end tangent handle |

### Tool-Generated ID Patterns

The rectangle tool, slot tool, and pattern tools generate entity IDs with a
shared base ID and semantic suffixes:

| Tool | Entity ID pattern |
| ---- | ----------------- |
| Rectangle | `{base}.bottom`, `{base}.top`, `{base}.left`, `{base}.right` |
| Slot | `{base}.0.startCap`, `{base}.0.endCap`, `{base}.0.left`, `{base}.0.right` |
| Linear pattern | `{base}.0.C.0` where C = instance index (1-based) |
| Circular pattern | `{base}.pattern.0.C.0` where C = instance index (1-based) |
| Mirror | `{base}.MirrorC` |

## Tool Decompositions

### Rectangle

Generates 4 line entities and 9 automatic constraints:

- 4 `COINCIDENT` constraints connecting corners (`.corner0` through `.corner3`)
- 1 `PERPENDICULAR` between adjacent sides
- 2 `PARALLEL` between opposite sides
- 1 `HORIZONTAL` on one side
- 1 `COINCIDENT` snap to a reference point (`.firstSnap0`)

User then adds `LENGTH` constraints for width/height.

### Slot

Generates 5 entities from a center line:

1. Center line (`BTMSketchCurveSegment-155`, `BTCurveGeometryLine-117`)
2. Start cap (semicircular arc, `BTCurveGeometryCircle-115`, `clockwise: true`)
3. End cap (semicircular arc)
4. Left offset line
5. Right offset line

Plus 12 internal constraints (all with `sketchToolType: "SLOT"`):

- 2 `COINCIDENT`: cap centers at center line endpoints
- 2 `OFFSET`: side lines offset from center line (`localOffset` + `localMaster`)
- 4 `COINCIDENT`: cap endpoints to side line endpoints (`.c1` through `.c4`)
- 4 `TANGENT`: caps tangent to side lines (`.t1` through `.t4`)
- 1 `DIAMETER`: slot width applied to a cap arc

### Linear Pattern

The `LINEAR_PATTERN` constraint references entities via a 3-index scheme:

```text
localInstance{G},{C},{R}
```

- G = entity group index (0-based, e.g. 0 = circle, 1 = center point)
- C = column/instance in direction 1 (0 = seed)
- R = row/instance in direction 2

Key parameters:

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `patternc1` | integer | Count in direction 1 |
| `patternc2` | integer | Count in direction 2 |
| `patterng` | integer | Number of entity groups |
| `localDirection1` | string | Construction direction line entity |

A construction direction line (with `isConstruction: true`) connects the seed
to the first instance, and a `LENGTH` constraint on it sets the spacing.

### Circular Pattern

The `CIRCULAR_PATTERN` constraint uses a 2-index scheme:

```text
localInstance{G},{C}
```

- G = entity group index
- C = angular instance index (0 = seed)

Key parameters:

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `patternc1` | integer | Count around circle |
| `patterng` | integer | Number of entity groups |
| `openPattern` | boolean | `false` = equal spacing around 360 degrees |
| `localPivot` | string | Center point entity for rotation |

### Mirror

Creates mirrored copies of entities. The mirrored entity gets ID
`{mirrorOpId}.MirrorC`. The `MIRROR` constraint uses:

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `localFirst` | string | Original entity |
| `localSecond` | string | Mirrored entity |
| `localMirror` | string | Mirror axis (construction line) |
| `sketchToolType` | enum | `"MIRROR"` |

Mirrored circles have `clockwise` and `xDir` negated relative to the original.

## Constraints

Constraints live in the sketch's `constraints` array as `BTMSketchConstraint-2`
entries.

### Essential Pattern

Constraints reference their target sketch entities through **`BTMParameterString-149`
entries in the `parameters` array**, not through a top-level `entityIds` field.

The `entityId` field (singular) on `BTMSketchConstraint-2` is the constraint's **own
identity** (inherited from `BTMSketchEntity-3`), not a reference to target entities.

### Inheritance Chain

```text
BTMNode-19                     -> { nodeId }
  BTMSketchEntity-3            -> { entityId, parameters[], namespace, name, index }
    BTMSketchConstraint-2      -> { constraintType, drivenDimension, helpParameters[], ... }
```

### Entity Reference Mechanism

Target entities are specified via `BTMParameterString-149` parameters:

| `parameterId` | Description |
| -------------- | ----------- |
| `localFirst` | Entity ID of the first constrained entity |
| `localSecond` | Entity ID of the second constrained entity (for 2-entity constraints) |
| `localMirror` | Mirror axis entity (for `MIRROR` constraints) |
| `localEntity1` | Point entity (for `MIDPOINT` — replaces `localFirst`) |
| `localEntity2` | Line entity (for `MIDPOINT` — replaces `localSecond`) |
| `localOffset` | Offset entity (for `OFFSET` — replaces `localFirst`) |
| `localMaster` | Master entity (for `OFFSET` — replaces `localSecond`) |
| `externalSecond` | `BTMParameterQueryList-148` referencing geometry outside the sketch |

Dimensional values are specified via `BTMParameterQuantity-147` in the same
`parameters` array.

### Referencing External Geometry

To reference geometry outside the sketch (e.g. the origin point), use
`BTMParameterQueryList-148` with parameterId `externalSecond` instead of
`localSecond`:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "externalSecond",
  "queries": [
    {
      "btType": "BTMIndividualQuery-138",
      "deterministicIds": ["IB"]
    }
  ]
}
```

### Constraint Types (`GBTConstraintType`)

| Type | Entities | Notes |
| ---- | -------- | ----- |
| `HORIZONTAL` | 1 (line) | Makes a line horizontal |
| `VERTICAL` | 1 (line) | Makes a line vertical |
| `COINCIDENT` | 2 (points or point+line) | Makes entities coincident |
| `EQUAL` | 2 (lines) | Makes lines equal length |
| `LENGTH` | 1 (line) + dimension | Sets line length |
| `DIAMETER` | 1 (circle) + dimension | Sets full circle diameter |
| `MAJOR_DIAMETER` | 1 (ellipse) + dimension | Sets ellipse major diameter |
| `MINOR_DIAMETER` | 1 (ellipse) + dimension | Sets ellipse minor diameter |
| `DISTANCE` | 2 (entities) + dimension | Sets distance between entities |
| `FIX` | 1 (any) | Pins entity in place |
| `PERPENDICULAR` | 2 (lines) | Makes lines perpendicular |
| `PARALLEL` | 2 (lines) | Makes lines parallel |
| `TANGENT` | 2 (curves) | Makes curves tangent; needs `helpParameters` |
| `CONCENTRIC` | 2 (arcs/circles) | Makes arcs share center |
| `MIDPOINT` | 2 (point + line) | Uses `localEntity1`/`localEntity2`, not `localFirst`/`localSecond` |
| `MIRROR` | 3 (original + copy + axis) | Uses `localFirst`, `localSecond`, `localMirror` |
| `OFFSET` | 2 (offset + master) | Uses `localOffset`/`localMaster` |
| `LINEAR_PATTERN` | N (instances) | See [Linear Pattern](#linear-pattern) for instance mapping |
| `CIRCULAR_PATTERN` | N (instances) | See [Circular Pattern](#circular-pattern) for instance mapping |

Full enum also includes: `NONE`, `NORMAL`, `PROJECTED`, `PIERCE`, `QUADRANT`,
`SILHOUETTED`, `CENTERLINE_DIMENSION`, `INTERSECTED`, `RHO`, `EQUAL_CURVATURE`,
`BEZIER_DEGREE`, `FREEZE`, `RADIUS`, `ANGLE`, `UNKNOWN`.

### Example Constraint JSON

#### HORIZONTAL (single entity)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "HORIZONTAL",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1"
    }
  ]
}
```

#### COINCIDENT (two local entities)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "COINCIDENT",
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

#### COINCIDENT with external geometry (e.g. origin)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "COINCIDENT",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1.start"
    },
    {
      "btType": "BTMParameterQueryList-148",
      "parameterId": "externalSecond",
      "queries": [
        {
          "btType": "BTMIndividualQuery-138",
          "deterministicIds": ["IB"]
        }
      ]
    }
  ]
}
```

#### EQUAL (two entities)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "EQUAL",
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

#### LENGTH (dimension on single entity)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "LENGTH",
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
    },
    {
      "btType": "BTMParameterEnum-145",
      "parameterId": "direction",
      "enumName": "DimensionDirection",
      "value": "MINIMUM"
    },
    {
      "btType": "BTMParameterEnum-145",
      "parameterId": "alignment",
      "enumName": "DimensionAlignment",
      "value": "ALIGNED"
    }
  ]
}
```

#### DIAMETER (full circle dimension)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "DIAMETER",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "circle1"
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "length",
      "expression": "1 in",
      "isInteger": false
    }
  ]
}
```

#### MIDPOINT (point at midpoint of line)

Note: uses `localEntity1`/`localEntity2`, not `localFirst`/`localSecond`.

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "MIDPOINT",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localEntity1",
      "value": "point1"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localEntity2",
      "value": "line1"
    }
  ]
}
```

#### MIRROR (entity mirrored across axis)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "MIRROR",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "circle1"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localSecond",
      "value": "mirrorOp1.MirrorC"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localMirror",
      "value": "constructionLine1"
    },
    {
      "btType": "BTMParameterEnum-145",
      "parameterId": "sketchToolType",
      "enumName": "SketchToolType",
      "value": "MIRROR"
    }
  ]
}
```

#### TANGENT (curves tangent to each other)

Requires `helpParameters` with the tangent point parameter value:

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "TANGENT",
  "helpParameters": [2.228],
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "line1"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localSecond",
      "value": "circle1"
    }
  ]
}
```

### Degree-of-Freedom Analysis

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

### Discovering New Constraint Formats

For constraint types not yet documented here, create the constraint manually in
the Onshape UI, then call `getPartStudioFeatures` to inspect the resulting JSON.
This is the most reliable way to determine the correct `parameters` structure.

## Pitfalls

1. **Full circles use `BTMSketchCurve-4`**, not `BTMSketchCurveSegment-155`.
   They have no `startPointId`/`endPointId`/`startParam`/`endParam`.

2. **Dimension full circles with `DIAMETER`**, not `RADIUS` or `LENGTH`.
   Ellipses use `MAJOR_DIAMETER` and `MINOR_DIAMETER`.

3. **Line `pntX/pntY` is not the start point.** It is the point at parameter
   `t = 0`. Use `pnt + startParam * dir` to get the actual start point.

4. **All coordinates are in meters.** Dimensions can use expressions with units
   (e.g. `"1 in"`, `"50 mm"`), but entity geometry coordinates are always meters.

5. **Do not use `entityIds` (plural)** on constraints — This field does not
   exist on `BTMSketchConstraint-2`. The API silently drops it, resulting in
   constraints with `entityId: ""` that produce warnings: *"Some constraints
   are not applicable to the current external references and have not been
   solved."*

6. **Use `LENGTH`, not `DISTANCE`**, for a single-line dimension — `LENGTH`
   constrains one line's length. `DISTANCE` sets the distance between two
   entities.

7. **Always include entity reference parameters** — Without `localFirst`/
   `localSecond` (or the appropriate variant like `localEntity1`/`localEntity2`)
   in the `parameters` array, the constraint has no entity references and will
   be ignored by the solver.

8. **`MIDPOINT` uses `localEntity1`/`localEntity2`**, not
   `localFirst`/`localSecond`.

9. **`OFFSET` uses `localOffset`/`localMaster`**, not
   `localFirst`/`localSecond`.

10. **`TANGENT` requires `helpParameters`** — a float array with the tangent
    point parameter value on the curve. Without it the solver may not find the
    solution.

11. **Referencing external geometry** uses `externalSecond`
    (`BTMParameterQueryList-148`), not `localSecond` (`BTMParameterString-149`).
    The origin point deterministic ID is `"IB"`.

12. **Pattern and tool constraints include `sketchToolType`** — an enum
    parameter (`"PATTERN"`, `"SLOT"`, `"MIRROR"`) that marks internally-managed
    constraints.
