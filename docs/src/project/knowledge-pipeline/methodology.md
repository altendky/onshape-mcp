# Knowledge Pipeline Methodology

## Overview

Knowledge is built through a self-supervised loop: Claude generates reference tasks, a human models them in Onshape, Claude analyzes the results and attempts to recreate them via API, and failures drive knowledge updates.

The human is the expert in the loop — performing CAD modeling, reviewing recreation quality, and resolving questions that Claude cannot answer from the API alone.
Claude handles task generation, API interaction, analysis, and documentation.

## The Learning Loop

| Step | Actor | Action |
| --- | --- | --- |
| 1. Design | Claude | Generate reference task spec |
| 2. Create | Claude | Create Onshape document with metadata |
| 3. Model | Human | Build the part in the Onshape UI |
| 4. Signal | Human | Provide document URL or UUID in chat |
| 5. Analyze | Claude | Read feature tree and version history |
| 6. Attempt | Claude | Recreate in a separate throwaway document |
| 7. Review | Human | Evaluate quality of recreation attempt |
| 8. Learn | Claude | Diagnose failures, extract insights |
| 9. Document | Claude | Update knowledge layers and document notes |
| 10. Compile | Human-triggered | Produce compiled insight resources |

### Step 1: Design

Claude generates a reference task specification targeting specific CAD operations and complexity tiers defined in the [curriculum](reference-models.md#curriculum-design).

The task spec includes: what to model, which CAD concepts it exercises, expected complexity, and what knowledge gaps it is designed to probe.

Task batches are informed by failures from previous iterations — operations that Claude struggled with get prioritized in subsequent batches.

### Step 2: Create

Claude creates an Onshape document via the `createDocument` API endpoint and populates metadata fields per the [metadata schema](reference-models.md#metadata-schema).

The document description contains the machine-parseable task spec.
The workspace description is set to `awaiting modeling`.

### Step 3: Model

The human builds the part in the Onshape UI.
No special tooling or workflow changes required — standard Onshape modeling.

The feature tree and version history capture the process automatically, including the sequence of operations and any iterations or corrections the modeler makes.

### Step 4: Signal

The human provides the document URL or reference model UUID to Claude in conversation.
No metadata update required — Claude reads the current state directly.

### Step 5: Analyze

Claude reads the reference model through the MCP server:

- Feature tree via `getPartStudioFeatures`
- Version history via `getDocumentHistory`
- Element metadata via `getElementsInDocument`

Claude maps each feature to its understanding of the corresponding API calls, noting any unknowns or uncertainties.

### Step 6: Attempt

Claude creates a separate throwaway Onshape document and attempts to recreate the reference model via API calls through the MCP server.

A separate document is used to avoid polluting the reference model.
The throwaway document can be deleted after review.

### Step 7: Review

The human evaluates the recreation attempt for quality.

This is currently a manual process — the human compares the recreation against the reference model and communicates results to Claude in conversation.

Automating portions of this review is an [open question](open-questions.md#pipeline-mechanics).

### Step 8: Learn

Claude diagnoses differences between the reference and the attempt.
Failures are the primary learning signal — they reveal exactly where the knowledge base is insufficient.

Failure categories include:

- **Wrong API call** — Used the wrong endpoint or operation
- **Wrong payload** — Correct endpoint but incorrect BTType, parameter, or structure
- **Wrong sequence** — Correct operations in the wrong order
- **Missing prerequisite** — Did not know an intermediate step was required (e.g., querying transient IDs before referencing geometry)
- **Conceptual gap** — Misunderstood how Onshape models a particular CAD concept

Each failure maps to one or more knowledge layers that need updating.

### Step 9: Document

Claude updates the relevant knowledge layer files in `docs/src/knowledge/` based on what was learned.

Claude also writes analysis notes to the reference model's Onshape document:

- **Document notes** — Human-readable narrative of what was learned
- **Comments** — Per-feature annotations where relevant
- **Workspace description** — Updated to reflect pipeline status

### Step 10: Compile

Triggered manually by the human (in conversation, via script, or other mechanism).

Updated knowledge layers are compiled into integrated, LLM-optimized resources.
The compilation mechanism is an [open question](open-questions.md#pipeline-mechanics).

## Feedback Dynamics

The loop is self-correcting: if compiled knowledge leads Claude to make incorrect API calls in future iterations, those failures surface in step 7 and feed back into knowledge updates.

Claude's task designs (step 1) are also informed by the loop — by articulating assumptions about how CAD operations work, Claude exposes those assumptions to testing.
When assumptions are wrong for Onshape specifically, that is exactly the knowledge gap the pipeline needs to fill.

The human's role diminishes over time as the knowledge base matures, shifting from frequent review and correction to occasional validation and edge-case resolution.
