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
   - Fetch changed files list: `gh api repos/:owner/:repo/pulls/<N>/files`
     (`:owner` and `:repo` are auto-substituted by `gh` when run from within the repo; alternatively, retrieve them via `gh repo view --json owner,name -q '.owner.login + "/" + .name'`)
   - If the PR references an issue, fetch the issue details

3. **Checkout the PR branch locally:**
   - Run `gh pr checkout <N>` to fetch and switch to the PR branch
   - This provides local access to the exact file contents for accurate line references

4. **Analyze changes:**
   - Read each changed file locally to get exact content and indentation
   - Identify issues, improvements, or concerns
   - Note specific line numbers for inline comments

5. **Prepare inline comments:**
   - Use suggestion blocks (fenced code with `suggestion` as the language) to propose concrete fixes. Example comment body:

     ````markdown
     The function name is unclear. Consider:

     ```suggestion
     def fetch_user_metrics():
     ```

     Or if this is for admin users specifically:

     ```suggestion
     def fetch_admin_metrics():
     ```
     ````

   - Nitpicks are acceptable

6. **Determine review type:**
   - If there are ANY inline comments about issues or improvements: **REQUEST_CHANGES**
   - If everything looks good as-is with no comments needed: **APPROVE**
   - Never use COMMENT — reviews should drive clear action: if there is anything to consider (a noted bug, a suggestion, or a clarifying question), use REQUEST_CHANGES to block until addressed; otherwise APPROVE so the PR can move forward. COMMENT leaves PRs in an ambiguous state where feedback exists but no decision is made.

7. **Submit review:**
   - Create a file named `review_payload.json` in the current working directory with the review payload:

     ```json
     {
       "event": "REQUEST_CHANGES",
       "body": "Overall review summary describing the main findings.",
       "comments": [
         {
           "path": "src/utils/metrics.py",
           "line": 42,
           "body": "Consider renaming this function for clarity.\n\n```suggestion\ndef fetch_user_metrics():\n```"
         },
         {
           "path": "src/api/handlers.py",
           "line": 87,
           "body": "This error message should include the actual status code.\n\n```suggestion\nraise ApiError(f\"Request failed with status {response.status_code}\")\n```"
         }
       ]
     }
     ```

   - If there are no inline comments (i.e., approving a clean PR), use an empty array: `"comments": []`
   - Submit via:

     ```bash
     gh api repos/{owner}/{repo}/pulls/<N>/reviews --input review_payload.json
     ```

   - After successful submission, delete the payload file:

     ```bash
     rm review_payload.json
     ```

8. **Report completion:**
   - Provide the review URL
   - List the inline comments that were posted
