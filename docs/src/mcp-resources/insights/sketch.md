# Sketches

How to create and structure sketch features via the Onshape Features API.

This document was derived from analysis of the
[sketch reference V2](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/v/98e9ea3fc5ac9c64559488b9/e/e138994dacce6d9757e6fc2b)
Part Studio, which contains 8 hand-crafted reference sketches covering lines,
circles, arcs, construction geometry, complex profiles, splines, ellipses,
patterns, offset planes, points, and slots. Additional constraint types
(`ANGLE`, `CENTERLINE_DIMENSION`) and visibility requirements were derived from
the [crayon profile exercise](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/w/c25e3f6fa28d58a69e60d91d/e/7937799504a65744e79b713e),
a revolve half-profile sketch using constraint-driven dimensioning. The
`disableImprinting` parameter, `ANGLE` boolean behaviors, sketch region
verification techniques, and the `updateFeatures` endpoint for constraint
parameter iteration were also derived from this exercise.

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

### Sketch-Level Parameters

The `parameters` array on a sketch contains the plane selection and optional
behavioral flags:

| `parameterId` | Type | Description |
| ------------- | ---- | ----------- |
| `sketchPlane` | `BTMParameterQueryList-148` | The plane or face the sketch is on (required) |
| `disableImprinting` | `BTMParameterBoolean-144` | When `true`, prevents this sketch's edges from being split by edges of other coplanar sketches. Default is `false` (imprinting enabled). See Pitfall #19 under [Pitfalls](#pitfalls). |

```json
{
  "parameters": [
    {
      "btType": "BTMParameterQueryList-148",
      "parameterId": "sketchPlane",
      "queries": [{ "btType": "BTMIndividualQuery-138", "deterministicIds": ["JCC"] }]
    },
    {
      "btType": "BTMParameterBoolean-144",
      "parameterId": "disableImprinting",
      "value": true
    }
  ]
}
```

Note: `disableImprinting` is not present by default on sketches created in the
UI. To add it via the API, use `updatePartStudioFeature` (which replaces the
entire feature definition) rather than `updateFeatures` (which can only update
parameters that already exist).

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
Construction entities are used as references and **do not generate sketch
regions**. Only non-construction entities (`isConstruction: false`) form the
closed profiles that produce sketch regions for downstream features like
extrude and revolve. If a line, arc, or circle that should be part of a
profile boundary is marked as construction, no sketch region will be produced
for that area. Note that **thin** extrude and revolve features can operate on
open (non-closed) sketch profiles without requiring a sketch region, adding
material thickness to one or both sides of the open section.

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

Plus 13 internal constraints (all with `sketchToolType: "SLOT"`):

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
This field must be a **non-empty unique string** for the constraint to render
visually in the sketch editor (e.g. perpendicular symbol, parallel bars,
coincident dot, dimension annotations). Without it, the constraint is
functionally active in the solver but invisible in the UI.

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

### Dimension Expressions

Dimensional values use `BTMParameterQuantity-147` with an `expression` string.
**Units are required** for measurement dimensions (length, angle, distance) — a
bare number like `"3.5"` is not valid for these parameters. Some positioning
parameters like `labelRatio` are unitless and accept bare numbers (e.g.
`"-2.0"`).

```json
{ "btType": "BTMParameterQuantity-147", "parameterId": "length", "expression": "3.5 in", "isInteger": false }
```

Supported expression formats:

| Format | Example |
| ------ | ------- |
| Decimal with unit | `"3.5 in"`, `"50 mm"`, `"20 deg"` |
| Fraction with unit | `"(5/16) in"`, `"(3/8) mm"` |
| Arithmetic | `"7 in + 3/4 in"` |
| Mixed units | `"5 in + 2 mm"` (valid but usually indicates variables are appropriate) |

The `value` field on `BTMParameterQuantity-147` is ignored on input; only
`expression` is used by the solver.

Most dimensional constraints use `parameterId: "length"` for their value. The
exception is `ANGLE`, which uses `parameterId: "angle"`.

### Dimension Label Positioning

Dimensional constraints require **label positioning parameters** for their
annotations (arrows, text, leader lines) to render in the sketch editor.
Without these parameters, the dimension affects the solver but no annotation
appears.

| Constraint Type | Label Parameters |
| --------------- | ---------------- |
| `LENGTH`, `DISTANCE`, `CENTERLINE_DIMENSION`, `DIAMETER` | `labelRatio` + `labelDistance` |
| `ANGLE` | `labelAngle` + `labelDistance` |

- `labelRatio` — position along the constrained entity (unitless; 0.5 = midpoint)
- `labelDistance` — perpendicular offset from the entity (expression with units, e.g. `"0.01*m"`)
- `labelAngle` — angular position for angle dimension labels (expression with units, e.g. `"0.38*rad"`)

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
| `ANGLE` | 2 (lines) + angle dimension | Uses `parameterId: "angle"` (**not** `"length"`); needs `clockwise`, `aligned`, `flipped` booleans |
| `CENTERLINE_DIMENSION` | 2 (centerline + entity) + dimension | Diameter dimension from a centerline; value is the **full diameter** |
| `LINEAR_PATTERN` | N (instances) | See [Linear Pattern](#linear-pattern) for instance mapping |
| `CIRCULAR_PATTERN` | N (instances) | See [Circular Pattern](#circular-pattern) for instance mapping |

Full enum also includes: `NONE`, `NORMAL`, `PROJECTED`, `PIERCE`, `QUADRANT`,
`SILHOUETTED`, `INTERSECTED`, `RHO`, `EQUAL_CURVATURE`,
`BEZIER_DEGREE`, `FREEZE`, `RADIUS`, `UNKNOWN`.

### Example Constraint JSON

#### HORIZONTAL (single entity)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "HORIZONTAL",
  "entityId": "cHorizontal1",
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
  "entityId": "cCoincident1",
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
  "entityId": "cCoincident2",
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
  "entityId": "cEqual1",
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
  "entityId": "cLength1",
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
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelRatio",
      "expression": "0.5",
      "isInteger": false
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelDistance",
      "expression": "0.01*m",
      "isInteger": false
    }
  ]
}
```

#### DIAMETER (full circle dimension)

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "DIAMETER",
  "entityId": "dCircle1",
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
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelRatio",
      "expression": "0.5",
      "isInteger": false
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelDistance",
      "expression": "0.01*m",
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
  "entityId": "cMidpoint1",
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
  "entityId": "cMirror1",
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
  "entityId": "cTangent1",
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

#### ANGLE (angle between two lines)

Note: uses `parameterId: "angle"` (**not** `"length"`), and requires three
boolean parameters. This is the only dimensional constraint that does not use
`"length"` as its value parameterId.

##### Boolean parameters

The `ANGLE` constraint has three boolean parameters that control which of the
four possible angles between two lines is constrained:

| Parameter | Effect |
| --------- | ------ |
| `clockwise` | Controls which side of `localSecond` the angle is measured from. **This is the critical parameter** — setting it incorrectly produces valid-looking sketches with silently wrong geometry (see Pitfall #18 under [Pitfalls](#pitfalls)). |
| `aligned` | Controls whether the angle is measured from the same-direction or opposite-direction vectors of the two lines. |
| `flipped` | Controls whether the supplementary angle (180° - angle) is used. |

**Determining the correct boolean values:** There is no formula to predict the
correct values from geometry alone — they depend on internal solver conventions
related to line direction vectors and endpoint ordering. The reliable approach
is:

1. Start with all three set to `false`
2. Evaluate the resulting face area or edge geometry via FeatureScript
   (see [Verifying Sketch Geometry](#verifying-sketch-geometry))
3. If the geometry is wrong (truncated edges, incorrect area), toggle
   booleans — start with `clockwise` as it has the most impact
4. The wrong values produce **no error** — the sketch status is "OK" but the
   topology is silently incorrect (see Pitfall #18 under [Pitfalls](#pitfalls))

##### Example

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "ANGLE",
  "entityId": "cAngle1",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "tipLine"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localSecond",
      "value": "centerline"
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "angle",
      "expression": "20 deg",
      "isInteger": false
    },
    {
      "btType": "BTMParameterBoolean-144",
      "parameterId": "clockwise",
      "value": true
    },
    {
      "btType": "BTMParameterBoolean-144",
      "parameterId": "aligned",
      "value": false
    },
    {
      "btType": "BTMParameterBoolean-144",
      "parameterId": "flipped",
      "value": false
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelAngle",
      "expression": "0.38*rad",
      "isInteger": false
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelDistance",
      "expression": "0.005*m",
      "isInteger": false
    }
  ]
}
```

#### CENTERLINE_DIMENSION (diameter from centerline)

Measures the distance from a centerline to another entity and displays the
**full diameter** (twice the perpendicular distance). Used for revolve profile
dimensioning. The `localFirst` entity is the centerline, `localSecond` is the
offset entity.

```json
{
  "btType": "BTMSketchConstraint-2",
  "constraintType": "CENTERLINE_DIMENSION",
  "entityId": "cDiameter1",
  "parameters": [
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localFirst",
      "value": "centerline"
    },
    {
      "btType": "BTMParameterString-149",
      "parameterId": "localSecond",
      "value": "outsideLine"
    },
    {
      "btType": "BTMParameterEnum-145",
      "parameterId": "direction",
      "enumName": "DimensionDirection",
      "value": "MINIMUM"
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "length",
      "expression": "(5/16) in",
      "isInteger": false
    },
    {
      "btType": "BTMParameterEnum-145",
      "parameterId": "halfSpace0",
      "enumName": "DimensionHalfSpace",
      "value": "LEFT"
    },
    {
      "btType": "BTMParameterEnum-145",
      "parameterId": "halfSpace1",
      "enumName": "DimensionHalfSpace",
      "value": "LEFT"
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelRatio",
      "expression": "-2.0",
      "isInteger": false
    },
    {
      "btType": "BTMParameterQuantity-147",
      "parameterId": "labelDistance",
      "expression": "0.012*m",
      "isInteger": false
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

### Updating Constraint Parameters

The `updateFeatures` endpoint (POST `/partstudios/d/{did}/w/{wid}/e/{eid}/features/updates`)
can update individual constraint parameters without resending the entire sketch
definition. Provide a partial `BTMSketch-151` with only the constraints to
modify, matched by `nodeId`:

```json
{
  "features": [{
    "btType": "BTMSketch-151",
    "featureId": "YOUR_FEATURE_ID",
    "constraints": [{
      "btType": "BTMSketchConstraint-2",
      "constraintType": "ANGLE",
      "entityId": "cAngle1",
      "nodeId": "NODE_ID_FROM_GET_FEATURES",
      "parameters": [
        { "btType": "BTMParameterBoolean-144", "parameterId": "clockwise", "value": true }
      ]
    }]
  }],
  "serializationVersion": "1.2.16"
}
```

This is much more efficient than `updatePartStudioFeature` when iterating on
constraint values (e.g., testing different `clockwise`/`aligned`/`flipped`
combinations). However, `updateFeatures` **cannot add new parameters** that
don't already exist on the feature — use `updatePartStudioFeature` for that
(e.g., adding `disableImprinting` for the first time).

## Verifying Sketch Geometry

Sketch constraint issues can be silent — the sketch status is `"OK"` but the
solved geometry is wrong (see Pitfall #18 under [Pitfalls](#pitfalls)). Use
`evalFeatureScript` to verify face areas and edge geometry after creating or
modifying a sketch.

### Counting faces and checking area

```javascript
function(context is Context, queries is map) {
    var sketch = qSketchFilter(
        qCreatedBy(makeId("YOUR_FEATURE_ID")),
        SketchObject.YES
    );
    var faces = qEntityFilter(sketch, EntityType.FACE);
    var faceArr = evaluateQuery(context, faces);
    var results = [];
    for (var i = 0; i < size(faceArr); i += 1) {
        results = append(results, {
            "index" : i,
            "area" : evArea(context, { "entities" : faceArr[i] })
        });
    }
    return { "numFaces" : size(faceArr), "faces" : results };
}
```

### Inspecting face boundary edges

```javascript
function(context is Context, queries is map) {
    var sketch = qSketchFilter(
        qCreatedBy(makeId("YOUR_FEATURE_ID")),
        SketchObject.YES
    );
    var faces = qEntityFilter(sketch, EntityType.FACE);
    var faceArr = evaluateQuery(context, faces);
    var faceEdges = qAdjacent(faceArr[0], AdjacencyType.EDGE, EntityType.EDGE);
    var edgeArr = evaluateQuery(context, faceEdges);
    var edgeData = [];
    for (var i = 0; i < size(edgeArr); i += 1) {
        var endpoints = evEdgeTangentLines(context, {
            "edge" : edgeArr[i], "parameters" : [0, 1]
        });
        edgeData = append(edgeData, {
            "index" : i,
            "length" : evLength(context, { "entities" : edgeArr[i] }),
            "start" : endpoints[0].origin,
            "end" : endpoints[1].origin
        });
    }
    return { "numEdges" : size(edgeArr), "edges" : edgeData };
}
```

Notes:

- Filter by `EntityType.FACE` to get real closed regions — sketch queries
  without this filter return degenerate zero-area entities as well.
- Use `qAdjacent` with `AdjacencyType.EDGE` to get edges bounding a specific
  face, rather than querying all sketch edges (which includes duplicates not
  adjacent to the face).
- Sketch coordinates map to 3D coordinates based on the sketch plane. For a
  Front plane sketch: sketch X → 3D X, sketch Y → 3D Z (with sign flip),
  all points have 3D Y = 0.

## Discovering Edge Deterministic IDs for Downstream Features

Downstream features like [Revolve](revolve.md) need to reference specific
sketch edges (e.g. the revolve axis). Sketch entity IDs (e.g. `"axisLine"`)
are sketch-internal identifiers that cannot be used in feature queries. You
must discover the edge's **deterministic ID** (transient ID) via
`evalFeatureScript`:

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

Each result includes the edge's `transientId` (inside the `query` field) along
with its start/end coordinates and length. Match these to your expected sketch
geometry to identify the correct edge.

**Duplicate edges:** This query returns **twice** as many edges as sketch
entities. Each sketch line appears as both a sketch entity edge and a face
boundary edge with different transient IDs but identical geometry. Either ID
works for downstream feature references. The sketch entity edges appear first
(lower indices).

Use the discovered `transientId` as a `deterministicIds` value in
`BTMIndividualQuery-138` when referencing the edge in downstream features.

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

13. **`ANGLE` uses `parameterId: "angle"`**, not `"length"` — This is the only
    dimensional constraint that does not use `"length"` as its value parameterId.
    Using `"length"` causes solver malfunction (OVERDEFINED status with
    underdefined entities). It also requires boolean parameters `clockwise`,
    `aligned`, and `flipped`.

14. **Use `CENTERLINE_DIMENSION`, not `DISTANCE`**, for revolve profile diameter
    dimensions — `CENTERLINE_DIMENSION` is purpose-built for measuring from a
    centerline and automatically represents the full diameter. `DISTANCE`
    between two parallel lines can cause solver issues.

15. **Constraints without `entityId` are invisible** — The solver uses them but
    the sketch editor shows no indicator (perpendicular symbol, parallel bars,
    coincident dot, etc.). Always provide a unique non-empty `entityId` string.

16. **Dimensions without label parameters don't render annotations** — Provide
    `labelRatio`/`labelDistance` for most dimension types, or
    `labelAngle`/`labelDistance` for `ANGLE` constraints, so the dimension text
    and arrows appear in the sketch editor.

17. **Construction entities do not produce sketch regions** — Entities with
    `isConstruction: true` are reference-only and are excluded from profile
    detection. Every line, arc, or circle that forms part of a closed profile
    boundary **must** have `isConstruction: false` for a sketch region to be
    generated. A common mistake is making an axis line construction when it
    also serves as one side of a revolve half-profile; the revolve will fail
    to find a region. (Thin extrude and thin revolve are an exception — they
    can operate on open sketch sections without a closed region, adding wall
    thickness to one or both sides of the open profile.)

18. **ANGLE boolean parameters fail silently** — Setting
    the wrong `clockwise`, `aligned`, or `flipped` values on an `ANGLE`
    constraint does **not** produce a solver error. The sketch status remains
    `"OK"` and all constraints appear satisfied. However, the resulting
    topology can be silently wrong: edges may be truncated to a fraction of
    their expected length, and face regions may cover only part of the
    intended profile. The only way to detect this is to verify the solved
    geometry via FeatureScript (check face areas with `evArea`, edge lengths
    with `evLength`, or edge endpoints with `evEdgeTangentLines`). If a
    sketch face area doesn't match expectations, toggling ANGLE booleans —
    especially `clockwise` — is a likely fix.

19. **Sketch imprinting fragments regions across coplanar sketches** —
    When multiple sketches share the same plane, Onshape **imprints** their
    edges onto each other by default. This splits edges at intersection
    points, fragmenting what should be a single closed region into multiple
    degenerate entities (many with area ≈ 0). Set `disableImprinting: true`
    on **all** coplanar sketches that interfere with each other — setting it
    on only one sketch is not sufficient. See [Sketch-Level
    Parameters](#sketch-level-parameters) for the parameter format.
