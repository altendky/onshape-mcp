---
description: Review a GitHub pull request with inline comments
---

# Review Pull Request

**PR reference (if provided):** $1

## Process

1. **Identify the PR:**
   - If a PR number or URL was provided, use it
   - If not provided, run `gh pr list --state open` and ask the user which to review

2. **Gather PR context:**
   - Fetch PR metadata: `gh pr view <N> --json title,body,baseRefName,headRefName,headRefOid`
   - Fetch the diff: `gh pr diff <N>`
   - Fetch changed files list: `gh api repos/{owner}/{repo}/pulls/<N>/files`
   - If the PR references an issue, fetch the issue details

3. **Checkout the PR branch locally:**
   - Run `gh pr checkout <N>` to fetch and switch to the PR branch
   - This provides local access to the exact file contents for accurate line references

4. **Analyze changes:**
   - Read each changed file locally to get exact content and indentation
   - Identify issues, improvements, or concerns
   - Note specific line numbers for inline comments

5. **Prepare inline comments:**
   - For each comment with a `suggestion` block:
     - Read the exact original line(s) from the local checkout
     - Write the replacement with identical indentation
   - Alternative suggestions in a single comment are acceptable
   - Nitpicks are acceptable

6. **Determine review type:**
   - If there are ANY inline comments about issues or improvements: **REQUEST_CHANGES**
   - If everything looks good as-is with no comments needed: **APPROVE**
   - Never use COMMENT

7. **Submit review:**
   - Create JSON payload with:
     - `event`: "APPROVE" or "REQUEST_CHANGES"
     - `body`: Overall review summary
     - `comments`: Array of inline comments with path, line, and body
   - Submit via `gh api repos/{owner}/{repo}/pulls/<N>/reviews --input <file>`

8. **Report completion:**
   - Provide the review URL
   - List the inline comments that were posted
