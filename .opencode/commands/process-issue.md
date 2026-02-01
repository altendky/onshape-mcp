---
description: Research, assess, and address a GitHub issue
---

# Process GitHub Issue

**Issue reference (if provided):** $1

## Process

1. **Identify the issue:**
   - If an issue number or URL was provided, fetch its details
   - If not provided, run `/issues` to list open issues and ask the user which to process

2. **Validate input type:**
    - If the input contains `#discussion_r` or `#r` → **stop** and inform the user: "This appears to be a PR review comment URL. Use `/process-comment` instead."
    - If the input URL contains `/pull/` → **stop** and inform the user: "This appears to be a pull request URL, not an issue. Use `/review-pr` to review PRs, or `/process-comment` for PR review comments."
    - If given a number, run `gh issue view <N> --json url --jq '.url'` and check the returned URL:
      - If the command fails (non-zero exit) → **stop** and ask the user: "Could not view issue `#N` — ensure the issue exists and you have access, or provide a full issue URL."
      - If it contains `/pull/` → **stop** and inform the user: "Number `#N` refers to a pull request, not an issue. Use `/review-pr` to review it."
      - If it contains `/issues/` → proceed

3. **Retrieve issue details:**
   - Get the full issue description, comments, and any linked context

4. **Research the claim:**
   - Investigate the validity of the issue against authoritative sources
   - Check relevant code, documentation, specs, or external references

5. **Assess validity:**
   - Determine if the issue is valid, partially valid, or invalid
   - Note any nuances (e.g., convention vs requirement)

6. **Present findings:**
   - Summarize research results
   - Provide a recommendation (fix, close, needs clarification, etc.)
   - Ask the user for their decision before proceeding

7. **Create a plan:**
   - Identify all files and locations that need changes
   - Outline specific tasks
   - Wait for user approval

8. **Check repository status:**
   - Identify the default branch (typically `main` or `master`)
   - If currently on a non-default branch, ask the user if this is intentional before proceeding
   - If there are uncommitted changes, ask the user how to proceed before continuing

9. **Execute:**
   - Switch to the default branch and pull latest changes
   - Create a new branch from the updated default branch
   - Make the changes
   - Commit with a message referencing the issue (e.g., "Closes #N")
   - Push the branch
   - Create a PR

10. **Report completion:**
    - Provide the PR URL
