---
description: Review GitHub issues labeled 'waiting' to check if blocking conditions are resolved
---

# Review Waiting Issues

**Issue number (if provided):** $1

## Process

1. **Fetch issues:**
   - If an issue number was provided, fetch that specific issue:
     `gh issue view <number> --json number,title,body,labels`
     - Verify this issue has the "waiting" label; skip if it doesn't
   - If no issue number provided, fetch all issues with the "waiting" label:
     `gh issue list --label waiting --json number,title,body,labels`

2. **For each issue, identify the blocking condition:**
   - Parse the issue body to find:
     - Upstream PR links (e.g., `github.com/<org>/<repo>/pull/<number>`)
     - Upstream release references (e.g., "once v1.2.3 is released")
     - Specific commits mentioned (e.g., merge commits to check for)
     - "How to check" or "Action Items" sections with explicit instructions
   - Record what the issue is waiting for

3. **Check if blocking conditions are resolved:**
   - For upstream PRs: `gh pr view <url> --json state,merged` to check if merged
   - For releases: identify the exact tag to check (prefer explicit tag from the issue body)
   - For commits in releases: `gh api repos/<org>/<repo>/compare/<tag>...<commit>` → if status is "behind" or "identical", the commit is included in that release
   - Follow any explicit "How to check" instructions from the issue body

4. **Report status for each issue:**
   - Format: `#<number>: <title>`
   - **Ready to process:** Blocking condition is resolved
     - Explain what changed (e.g., "PR #123 was merged", "v1.2.3 was released")
     - List the "Action Items" from the issue body
   - **Still waiting:** Blocking condition not yet resolved
     - Explain what is still blocking (e.g., "PR #123 is still open", "No new release since v0.22.0")

## Output format

```markdown
## Waiting Issues Status

### Ready to Process
- #45: Use prebuilt cargo-nextest for ARM musl when available
  - Blocking condition resolved: [explanation]
  - Action items: [from issue body]

### Still Waiting
- #42: Update lychee pre-commit hook to release once PR #2002 is included
  - Still waiting for: [explanation]
```

If all issues are ready, note that. If all are still waiting, note that too.

## Notes

- This command assumes issues reference GitHub-hosted repositories. Support for other Git hosting platforms (GitLab, Bitbucket, etc.) may be needed in the future.
