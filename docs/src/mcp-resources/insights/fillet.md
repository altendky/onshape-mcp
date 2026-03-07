# Fillet

How to create fillet features via the Onshape Features API to round edges on
solid bodies.

This document was derived from the
[coffee mug exercise](https://cad.onshape.com/documents/31a1864ce146242e0041ada7/w/c25e3f6fa28d58a69e60d91d/e/25cbc4078053af59e2007f6c),
where fillets were applied to the outer bottom edge and inner bottom edge of a
revolved mug body.

## Fillet Workflow

Filleting is simpler than extrude, revolve, or sweep because there is no sketch
involved. The workflow is:

1. **Create the solid body** — using extrude, revolve, sweep, or other features
2. **Discover edge deterministic IDs** — via `evalFeatureScript`
3. **Create the fillet** — referencing the discovered edge IDs

### Discovering edges to fillet

After creating a solid body, discover its edges via `evalFeatureScript`:

```javascript
function(context is Context, queries is map) {
    var edges = evaluateQuery(context, qCreatedBy(makeId("<featureId>"), EntityType.EDGE));
    var edgeData = [];
    for (var i = 0; i < size(edges); i += 1) {
        edgeData = append(edgeData, {
            "index" : i,
            "query" : edges[i],
            "length" : evLength(context, { "entities" : edges[i] })
        });
    }
    return edgeData;
}
```

For revolved bodies, circular edges can be identified by their circumference:
a circle of radius `r` has length `2 * pi * r`. For example, a revolved body
with outer radius 42.5 mm has circular edges of length ~267 mm.

## Feature Structure

```json
{
  "btType": "BTMFeature-134",
  "featureType": "fillet",
  "name": "My Fillet",
  "parameters": [
    { "entities": "«edge query list»" },
    { "radius": "«expression»" }
  ]
}
```

## Parameters

### `entities` — Edges or faces to fillet

References the edges to round. Multiple edges can be included in a single
fillet feature if they share the same radius:

```json
{
  "btType": "BTMParameterQueryList-148",
  "parameterId": "entities",
  "queries": [
    { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<edge-id-1>"] },
    { "btType": "BTMIndividualQuery-138", "deterministicIds": ["<edge-id-2>"] }
  ]
}
```

Faces can also be selected — all edges of the face are filleted.

### `radius` — Fillet radius

The radius of the fillet arc. Accepts unit expressions.

```json
{
  "btType": "BTMParameterQuantity-147",
  "parameterId": "radius",
  "expression": "3 mm",
  "isInteger": false
}
```

The radius must be small enough that the fillet does not consume adjacent faces.
A radius larger than the smallest adjacent face dimension will cause the fillet
to fail.

### Additional parameters

| parameterId | Type | Description |
| ----------- | ---- | ----------- |
| `isAsymmetric` | boolean | Enable different distances on each side |
| `rhoOrMagnitude` | quantity | Cross-section shape control (0–1, default 0.5) |
| `isVariable` | boolean | Enable variable radius along an edge |
| `isTangentPropagation` | boolean | Propagate fillet to tangent-connected edges |

## Complete Example: Fillet Bottom Edge

This example fillets the bottom outer edge of a revolved mug body with a 3 mm
radius:

```json
{
  "feature": {
    "btType": "BTMFeature-134",
    "featureType": "fillet",
    "name": "Fillet - Bottom Edge",
    "parameters": [
      {
        "btType": "BTMParameterQueryList-148",
        "parameterId": "entities",
        "queries": [
          {
            "btType": "BTMIndividualQuery-138",
            "deterministicIds": ["<bottom-edge-id>"]
          }
        ]
      },
      {
        "btType": "BTMParameterQuantity-147",
        "parameterId": "radius",
        "expression": "3 mm",
        "isInteger": false
      }
    ]
  },
  "serializationVersion": "1.2.16",
  "sourceMicroversion": "<microversion-from-previous-call>"
}
```

## Pitfalls

1. **Edge IDs require discovery** — Like sketch regions and revolve axes, fillet
   edge IDs must be discovered via `evalFeatureScript`. You cannot reference
   edges by name or index.

2. **Use separate fillets for different radii** — All edges in a single fillet
   feature share the same radius (unless `isVariable` is enabled). To apply
   different radii to different edges, create multiple fillet features.

3. **Fillet order matters** — Filleting changes the topology of the body. Edge
   IDs discovered before a fillet may no longer be valid after it — edges can
   be split, merged, or consumed. Deterministic IDs are only valid for one
   microversion, and each `addPartStudioFeature` call creates a new
   microversion. If you need to fillet multiple edges with different radii,
   re-discover edge IDs between each fillet operation.

4. **Radius must not exceed adjacent geometry** — A fillet radius larger than
   the smallest face adjacent to the edge will fail. For thin walls (e.g. 4 mm
   wall thickness on a mug), the fillet radius must be less than 4 mm.

5. **Feed `sourceMicroversion` forward** — Same as other features. Each
   `addPartStudioFeature` response includes a `sourceMicroversion`. Use it in
   subsequent calls.
