# Knowledge Pipeline Architecture

## Overview

Knowledge is maintained in four source layers, each building on the previous.
The source layers are optimized for maintainability and clear reasoning.
A separate compilation step produces integrated, token-efficient resources for LLM consumption.

```text
                        ┌───────────────────────────┐
                        │     Reference Models       │
                        │    (Onshape documents)     │
                        └─────────────┬─────────────┘
                                      │
                        ┌─────────────▼─────────────┐
                        │   Self-Supervised Loop     │
                        │   (analyze, attempt,       │
                        │    review, learn)          │
                        └─────────────┬─────────────┘
                                      │
           ┌──────────────────────────▼──────────────────────────┐
           │            Layered Source Knowledge                  │
           │                                                     │
           │   L1: Generic CAD Concepts                          │
           │   L2: Onshape Domain Model                          │
           │   L3: API Mechanics                                 │
           │   L4: Workflow Patterns                              │
           └──────────────────────────┬──────────────────────────┘
                                      │
                        ┌─────────────▼─────────────┐
                        │     Compilation            │
                        │     (manual trigger)       │
                        └─────────────┬─────────────┘
                                      │
           ┌──────────────────────────▼──────────────────────────┐
           │       Compiled Insight Resources                     │
           │       (task-oriented, LLM-optimized)                │
           └─────────────────────────────────────────────────────┘

           ┌─────────────────────────────────────────────────────┐
           │       Manual Insight Resources                       │
           │       (existing: sketch, extrude, shaded views)     │
           │       (independent of pipeline)                     │
           └─────────────────────────────────────────────────────┘
```

## Knowledge Layers

### Layer 1: Generic CAD Concepts

Foundational CAD knowledge portable across CAD systems: topology (faces, edges, vertices), sketch constraint systems, feature operations (extrude, revolve, sweep, loft, fillet, chamfer, pattern, mirror, boolean, shell, draft), assembly mating concepts, coordinate systems.

This layer is aspirationally shareable with other CAD API integration projects but is not over-engineered for reuse.
Clean separation of concerns within this project is the primary goal.

**Location:** [`docs/src/knowledge/generic-cad/`](../../knowledge/generic-cad/index.md)

### Layer 2: Onshape Domain Model

How Onshape implements generic CAD concepts.
Part studios vs. assemblies vs. drawings, feature tree semantics, the BTType system, configurations, contexts, the document/workspace/version/element model, element types, reference geometry.

This layer bridges from generic CAD understanding to Onshape-specific abstractions.

**Location:** [`docs/src/knowledge/onshape-domain/`](../../knowledge/onshape-domain/index.md)

### Layer 3: API Mechanics

Onshape REST API specifics: endpoint signatures, payload structures, ID resolution patterns (document/workspace/element IDs, transient IDs, deterministic IDs), authentication, error handling, query parameter conventions.

Partially derivable from the OpenAPI spec, but the spec alone does not capture the domain knowledge needed to use endpoints correctly.

**Location:** [`docs/src/knowledge/api-mechanics/`](../../knowledge/api-mechanics/index.md)

### Layer 4: Workflow Patterns

Idiomatic sequences for common tasks.
Each pattern references concepts from all three layers above: what CAD operation is being performed (L1), how Onshape models it (L2), and what API calls implement it (L3).

Examples: "to fillet an edge, create a fillet feature referencing edge transient IDs obtained from a topology query", "to create a sketch on a face, use a mate connector subFeature targeting the face".

Encodes ordering constraints, common compositions, and error recovery strategies.

**Location:** [`docs/src/knowledge/workflow-patterns/`](../../knowledge/workflow-patterns/index.md)

## Compilation

The layered source knowledge is the canonical representation — optimized for maintainability, auditability, and clear reasoning about why things are done a certain way.

A compilation step produces integrated, per-topic resources optimized for LLM context windows: flattened, cross-referenced, redundancy removed.
This is analogous to how compiled code works — you keep the source hierarchy for development but ship a different form for execution.

Compilation is triggered manually.
The compiled output is a build artifact that can be regenerated from the source layers.

The compilation mechanism, output format, and serving strategy are [open questions](open-questions.md#pipeline-mechanics).
