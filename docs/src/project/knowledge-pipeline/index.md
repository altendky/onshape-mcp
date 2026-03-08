# Knowledge Pipeline

A self-supervised methodology for building layered CAD API knowledge that can be compiled into LLM-optimized reference material.

## Motivation

The Onshape API is large and domain-specific.
An LLM cannot reliably use it from endpoint signatures alone — it needs contextual knowledge about CAD concepts, Onshape's data model, payload structures, and idiomatic operation sequences.
This pipeline systematically builds that knowledge through a loop of expert demonstration, automated analysis, and iterative refinement.

## Relationship to Manual Insights

The existing [Insights](../../mcp-resources/insights/index.md) resources (sketch, extrude, shaded views) were authored manually and remain independent of this pipeline.
The pipeline produces its own compiled output.
Whether compiled output eventually supplements or replaces manual insights is an [open question](open-questions.md#knowledge-organization).

## Documentation

- [Architecture](architecture.md) — Knowledge layer model, compilation, artifact locations
- [Methodology](methodology.md) — The self-supervised learning loop, roles, step-by-step process
- [Reference Models](reference-models.md) — Identification scheme, metadata conventions, curriculum design
- [Open Questions](open-questions.md) — Unresolved design decisions

## Knowledge Base

The pipeline populates the [Knowledge Base](../../knowledge/index.md), a layered collection of source material organized for maintainability and clear reasoning.
