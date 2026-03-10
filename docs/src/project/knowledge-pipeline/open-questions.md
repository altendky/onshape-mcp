# Knowledge Pipeline Open Questions

## Pipeline Mechanics

- [ ] Recreation location — Currently planning separate throwaway documents for recreation attempts.
  Alternatives: same document with a new Part Studio tab, or a dedicated recreation workspace.
- [ ] Comparison automation — Currently human review for quality assessment.
  Future: automated geometric comparison, feature tree diffing, or hybrid approaches.
- [ ] Compilation mechanism — Manual trigger, specifics to be determined.
  Options: AI-driven in conversation, scripted tooling, hybrid.
- [ ] Compiled output location — Where do compiled insight resources live on disk?
  How do they integrate with the existing MCP resource serving mechanism (compile-time codegen from `docs/src/mcp-resources/`)?

## Knowledge Organization

- [ ] Manual vs. compiled insight coexistence — How do hand-written insights and pipeline-generated insights share the `mcp-resources/` space?
  Options: separate resource groups (e.g., `insights-generated:`), unified with provenance tags, compiled replaces manual over time.
- [ ] Contribution model — How other people's Onshape models enter the pipeline.
  Needs: discovery mechanism, quality criteria, metadata conventions for external contributions.
- [ ] Layer 1 portability — Whether generic CAD concepts are shared with other projects or only cleanly separated within this one.
- [ ] Document description format — Exact structured format for the machine-parseable task spec in the document description field.
  Must be both machine-parseable and reasonably human-readable in the Onshape UI.

## Onshape API Limitations

- [ ] Tags — The Onshape API silently drops tags on document creation/update.
  Need alternative categorization approach if tags are required for filtering or enumeration.
