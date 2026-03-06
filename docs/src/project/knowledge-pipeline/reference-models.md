# Reference Models

Reference models are Onshape documents that serve as ground truth for the [learning loop](methodology.md#the-learning-loop).
Each document is simultaneously a CAD model, a test case specification, and a learning log.

## Identification

Each reference model has two identifiers:

- **UUID** — Stable, universally unique.
  Stored in Onshape document metadata.
  Used for cross-referencing in the knowledge base.
  Never changes, even if the model is reorganized.
- **Hierarchy path** — Human-navigable structure that conveys intent and categorization at a glance.
  Can be reorganized without breaking UUID-based references.

Example: hierarchy path `ref/sketch/constraints/coincident` with UUID `a1b2c3d4-e5f6-7890-abcd-ef1234567890`.

The hierarchy provides browsable structure without imposing an artificial scale limit.
Depth and branching grow organically with the curriculum.

## Metadata Schema

Reference models use Onshape document metadata fields to store pipeline state and analysis results.
All fields are both visible in the Onshape UI and programmatically accessible via the API.

| Field | Onshape location | API | Purpose |
| --- | --- | --- | --- |
| Document name | Document properties | `updateDocumentAttributes` | Human-readable title including hierarchy path |
| Document description | Document details (10K chars) | `updateDocumentAttributes` | Machine-parseable task spec |
| Document notes | Left sidebar panel | `updateDocumentAttributes` | Human-readable analysis narrative |
| Workspace description | Workspace properties | `updateWVMetadata` | Pipeline status |
| Comments | Left sidebar comments | `createComment` | Per-feature annotations from analysis |

### Pipeline Status Values

The workspace description field tracks where a reference model is in the learning loop:

| Status | Meaning |
| --- | --- |
| `awaiting modeling` | Document created, task spec written, waiting for human to model |
| `modeled` | Human has completed the model |
| `analyzed` | Claude has read and analyzed the feature tree |
| `attempted` | Claude has attempted API recreation |
| `reviewed` | Human has reviewed the recreation attempt |
| `documented` | Knowledge layers have been updated from this reference |

### Document Description Format

The document description field contains a machine-parseable task specification.
The exact structured format is an [open question](open-questions.md#knowledge-organization) — it must be both machine-parseable and reasonably human-readable in the Onshape UI.

Required fields:

- UUID
- Hierarchy path
- Target CAD operations
- Complexity tier
- Coverage tags
- Original task prompt from Claude

## Curriculum Design

### Complexity Tiers

Reference models are organized by complexity to ensure foundational knowledge is established before tackling advanced patterns.

**Tier 1: Primitives** — Individual operations in isolation.
Sketch types, extrude, revolve, sweep, loft, fillet, chamfer, pattern, mirror, boolean, shell, draft.
Establishes the basic API call for each operation.

**Tier 2: Combinations** — Operations that interact.
Fillet after pattern vs. pattern after fillet, sketches referencing other features, multi-body workflows.
Reveals ordering dependencies and reference resolution patterns.

**Tier 3: Assemblies** — Multi-part compositions.
Fixed + one mate, multi-part constrained, sub-assemblies, configurations.
Introduces assembly-specific API patterns and cross-part references.

**Tier 4: Edge cases** — Patterns that tend to trip up API usage.
Reference geometry, derived features, in-context editing, configurations with suppression, import/export.
Surfaces Onshape-specific behaviors that diverge from generic CAD assumptions.

### Coverage Tracking

Each reference model maps to specific knowledge layer content it informs.
This mapping is recorded in the document metadata and in the knowledge layer files themselves.

Gaps are identified by comparing the curriculum's target coverage against the knowledge base's actual content.
Claude designs subsequent task batches to target identified gaps, with priority given to operations that caused failures in previous iterations.
