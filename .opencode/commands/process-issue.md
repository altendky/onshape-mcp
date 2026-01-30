---
description: Research, assess, and address a GitHub issue
---

# Process GitHub Issue

**Issue reference (if provided):** $1

## Process

1. **Identify the issue:**
   - If an issue number or URL was provided, fetch its details
   - If not provided, run `/issues` to list open issues and ask the user which to process

2. **Retrieve issue details:**
   - Get the full issue description, comments, and any linked context

3. **Research the claim:**
   - Investigate the validity of the issue against authoritative sources
   - Check relevant code, documentation, specs, or external references

4. **Assess validity:**
   - Determine if the issue is valid, partially valid, or invalid
   - Note any nuances (e.g., convention vs requirement)

5. **Present findings:**
   - Summarize research results
   - Provide a recommendation (fix, close, needs clarification, etc.)
   - Ask the user for their decision before proceeding

6. **Create a plan:**
   - Identify all files and locations that need changes
   - Outline specific tasks
   - Wait for user approval

7. **Check repository status:**
   - Identify the default branch (typically `main` or `master`)
   - If currently on a non-default branch, ask the user if this is intentional before proceeding
   - If there are uncommitted changes, ask the user how to proceed before continuing

8. **Execute:**
   - Switch to the default branch and pull latest changes
   - Create a new branch from the updated default branch
   - Make the changes
   - Commit with a message referencing the issue (e.g., "Closes #N")
   - Push the branch
   - Create a PR

9. **Report completion:**
   - Provide the PR URL
