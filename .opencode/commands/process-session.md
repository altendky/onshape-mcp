---
description: Review session for difficulty spots and improvement opportunities
---

# Process Session

Review the active session to identify where there was difficulty executing the task. Extract lessons learned and offer to capture them as project documentation improvements.

**Scoping (if any):** $ARGUMENTS

If scoping text is provided, focus the review on that subset of the session's work. Otherwise, review the entire session.

## Phase 1: Session Summary

Before analyzing for difficulty, establish the session's narrative.

### 1a. Build a session timeline

Walk through the full conversation (or scoped subset) and identify each **unit of work** — a coherent attempt at a subtask. For each unit, record:

- What was attempted
- Approximate tool call count
- Whether it succeeded on the first try or required iteration
- The outcome (succeeded, abandoned, pivoted, still in progress)

For units of work that required iteration or encountered failures, also record the **tool call sequence** — the ordered list of tool calls made, with brief notes on each call's purpose and result. This detail is essential for Phase 2 subagents to identify root causes, assess severity, and distinguish avoidable from inherent difficulties. A unit that "required 12 tool calls" is less actionable than knowing which calls were redundant, which returned errors, and where the approach pivoted.

### 1b. Write a session summary

Compose a concise summary (a few paragraphs) covering:

- **Objective:** What the user was trying to accomplish
- **Approach:** The overall strategy taken
- **Outcome:** What was achieved, what remains
- **Notable events:** Anything unusual — errors, surprises, pivots, breakthroughs

This summary, along with the session timeline and the scoped transcript, will be distributed to analysis subagents in Phase 2.

## Phase 2: Difficulty Identification

Launch one **subagent** per difficulty category, in parallel. Each subagent receives:

- The scoped transcript (the full conversation, or the scoped subset if scoping was specified)
- The session summary from Phase 1b
- The session timeline from Phase 1a
- The category definition (from the list below)
- The general analysis guidance (below)

Each subagent reviews the session for instances of its assigned difficulty category and returns findings.

### Difficulty Categories

**1. Research dead ends**
Exploration chains (grep, read, search, API search/explain, web fetch) that did not yield actionable results. Long sequences of looking at code or documentation that turned out to be irrelevant. Includes cases where the agent searched for something that was right in front of it, or searched broadly when a targeted query would have sufficed.

**2. Iterative correction**
Edit-fail-edit cycles. Changes that had to be reverted or redone. Repeated attempts at the same operation with slightly different approaches. Includes compilation or test failures caused by the agent's own changes that required multiple rounds to fix.

**3. API/tool friction**
Failed tool calls, unexpected API behavior, high volume of similar calls that could have been batched or avoided. Includes Onshape API calls that returned errors due to incorrect parameters, missing prerequisites, or misunderstood schemas. Also includes cases where the agent made many small reads instead of reading a larger context, or made redundant searches.

**4. Unclear requirements**
Back-and-forth with the user to clarify intent. Work that had to be redone after the user corrected a misunderstanding. Cases where the agent proceeded with an assumption that turned out to be wrong. Includes situations where the agent could have asked a clarifying question upfront but didn't.

**5. Knowledge gaps**
Areas where the agent lacked domain knowledge (Onshape API behavior, FeatureScript conventions, Rust idioms, project-specific patterns) and had to discover it through trial and error. The key signal is: the agent eventually learned something it needed, but the path to learning it was expensive. This is the most important category for generating documentation improvements.

**6. Approach changes**
Mid-task strategy pivots where significant work on a prior approach was discarded. Cases where the agent went deep into an implementation path before realizing it wouldn't work. Includes cases where a simpler approach existed but wasn't considered initially.

### General Analysis Guidance

These principles apply to all analysis subagents:

- **Focus on the expensive difficulties.** A single failed grep that was immediately corrected is not interesting. A 15-call exploration chain that ended in a dead end is. Weight findings by the cost of the difficulty (tool calls wasted, time spent, complexity of recovery).
- **Distinguish avoidable from inherent.** Some difficulties are inherent to the problem (e.g., an API genuinely has unclear documentation). Others are avoidable with better project context (e.g., the answer was already in an insight doc the agent didn't read, or a convention exists that wasn't documented). Flag which is which — avoidable difficulties are candidates for documentation improvements.
- **Include enough context for documentation.** Each finding should contain enough detail that someone writing a documentation update could understand the issue without re-reading the full session. Include: what the agent was trying to do, what went wrong, what the correct approach turned out to be, and why the correct approach works.
- **Per-finding data:**
  - Brief description of what happened
  - Why it was difficult (root cause)
  - Severity: **minor** (small friction, <2 extra tool calls), **moderate** (notable detour, 3-10 extra calls), **major** (significant time sink or complete dead end)
  - Avoidability: **avoidable** (better docs/context would have prevented it) or **inherent** (unavoidable given the current state of things)
  - If avoidable: what documentation or context would have prevented it
  - The lesson learned — what the agent now knows that it didn't before

## Phase 3: Presentation

Collect results from all analysis subagents and assemble the output.

### Output Structure

Present the results in this order:

1. **Per-category narratives** — for each difficulty category that produced findings, display a prose summary: what patterns were found, how severe they were, and what the root causes were. Categories with no findings are omitted.

2. **Clean categories** — a single line noting which categories had no findings.

3. **Lessons learned** — a consolidated list of all lessons extracted from findings across all categories. Each lesson is a concise statement of something the agent learned during the session that it didn't know before. Group by topic (e.g., "Onshape API", "Project conventions", "Rust/tooling").

4. **Executive summary table** — a compact table at the bottom. Columns: category, finding count, worst severity, avoidable count. Only categories with findings are included.

The executive summary table must be the **last thing printed** so it is always visible above the prompt.

## Phase 4: Improvement Suggestions

After presenting findings, propose concrete documentation improvements for each **avoidable** finding.

### Improvement Types

For each avoidable finding, suggest one or more of:

- **Insight document** (new or updated) — for Onshape API usage patterns, FeatureScript conventions, or workflow knowledge. These live in `docs/src/mcp-resources/insights/` and are served as MCP resources with the `insights:` URI prefix. This is the default suggestion for Onshape API or CAD domain knowledge.
- **AGENTS.md update** — for project-level conventions, architecture guidance, or development workflow knowledge that agents should always have access to.
- **Project documentation update** — for updates to existing docs in `docs/src/project/` that would help future sessions.
- **New command** — for workflows that were done manually but could be automated. These live in `.opencode/commands/`.
- **Other** — any improvement that doesn't fit the above categories. Describe what and where.

For each suggestion, provide:

- The target file (existing or new)
- A brief description of the content to add or change
- The lesson(s) it would capture

### User Confirmation

Present all improvement suggestions using the **question tool** with one question per suggestion:

- `header`: target file name (truncated to 30 chars)
- `question`: the full description of the proposed change, including the lesson(s) it would capture and a draft of the content to add
- `options`: `"Implement (Recommended)"`, `"Skip"`, `"Revise"`
- `multiple: false`

The user may type a custom answer as revision guidance. If guidance is given, revise the proposal and re-present until accepted or skipped.

## Phase 5: Implementation

For each accepted improvement, implement the change:

1. If the target file exists, read it first to understand the current structure.
2. Make the edit or create the new file.
3. If the change is to an insight document that should be registered as an MCP resource, check `docs/src/mcp-resources/insights/index.md` and update the listing if needed.
4. Show the diff to the user.

After all improvements are implemented, ask the user if they would like to commit the changes.

## Error Handling

- **Subagent failures:** Report which category's analysis failed. Continue with all other categories. Note the incomplete analysis in the final output and the executive summary.
- **Empty results:** If no difficulties are identified in any category, report that the session executed smoothly and skip Phases 4 and 5. This is a valid outcome.
- **Implementation failures:** If an edit fails (e.g., merge conflict with concurrent changes), show the error and ask the user how to proceed.
