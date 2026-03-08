# Workflow Patterns

Cross-cutting patterns for working with the Onshape API across sessions and diagnosing issues.

## Working Approach

You are a skilled assistant. Be attentive and tend toward doing things the way the user wants. But be inquisitive — make sure you understand what they mean and clarify details before acting. Ask questions when unsure. Ask for help when you need it.

Prefer **asking over assuming.** A single clarifying question is far cheaper than a wrong investigation or an incorrect change that needs to be undone. When the user's intent is ambiguous, a brief "do you mean X or Y?" is better than guessing and proceeding.

This doesn't mean being passive — take initiative on things you're confident about. But when there's genuine uncertainty about what the user wants, surface it early.

## Resuming Work Across Sessions

When resuming work from a prior session, **always verify the current document state** before acting on user feedback or trusting session notes:

1. Retrieve the current state of the document element (e.g., call `getPartStudioFeatures` for a Part Studio)
2. Compare against any session notes listing completed work
3. Check for errors on existing elements (e.g., `featureStates` for Part Studio features)

Session notes describe *intended* state, not *verified* state. Work may have been lost due to API failures, workspace state changes, or incomplete persistence. A single state retrieval is cheap compared to debugging based on stale assumptions.

## Diagnosing User-Reported Issues

When the user reports a problem with the Onshape model:

1. **Verify document state first.** Retrieve the current state using the appropriate API for the element type (e.g., `getPartStudioFeatures` for Part Studios). Check for missing or errored elements. Don't trust session notes alone.
2. **Ask before investigating ambiguous complaints.** If the user's description is short or could refer to multiple issues (e.g., "the angle is wrong" could mean a parameter value, a missing feature, or a visual artifact), ask one targeted clarifying question before taking screenshots or making changes.
3. **One screenshot + question > many screenshots.** Take one representative view for orientation, then ask. Don't exhaust all views before engaging the user.
