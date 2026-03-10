# Insights

Practical reference for correct patterns, API usage, and design guidelines.

## Onshape API

- [Shaded Views](shaded-views.md) — Rendering Part Studio images via the API, critical `pixelSize` parameter
- [Sketch](sketch.md) — Sketch entities, geometry types, constraints, planes, tool decompositions
- [Extrude](extrude.md) — Extrude feature parameters, sketch-to-solid workflow, region discovery
- [Revolve](revolve.md) — Revolve feature parameters, sketch-to-revolved-solid workflow, axis edge discovery
- [Sweep](sweep.md) — Sweep feature parameters, path + profile workflow, perpendicular plane requirement
- [Construction Plane](cplane.md) — cPlane creation modes, CURVE_POINT for sweep profiles, visual extent sizing
- [Fillet](fillet.md) — Fillet feature parameters, edge discovery, radius constraints
- [Part Studio](part-studio.md) — Feature error retrieval via evalFeatureScript, runtime debugging
- [FeatureScript](featurescript.md) — Feature Studio API, stdlib version discovery, import conventions, notices workaround
- [Debug Entities](debug-entities.md) — Accessing debug entity data via evalFeatureScript, wire body debug geometry for screenshots

## General

- [Workflow Patterns](workflow.md) — Session resumption, model state verification, diagnosing user-reported issues
