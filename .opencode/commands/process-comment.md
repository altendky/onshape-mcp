---
description: Process a GitHub PR review comment
---

# Process GitHub PR Review Comment

**Comment reference:** $ARGUMENTS

## Process

1. **Parse the input:**
   - If given a URL like `https://github.com/owner/repo/pull/123#discussion_r1234567890` or `https://github.com/owner/repo/pull/123/files#r1234567890`, extract the comment ID (the number after `r`)
   - If given just a comment ID number, use it directly

2. **Fetch the comment using gh api:**
   - Determine the repository owner/repo from the URL or from `gh repo view --json owner,name`
   - Fetch comment details: `gh api repos/{owner}/{repo}/pulls/comments/{comment_id}`
   - This returns the comment body, file path, line numbers, and diff context

3. **Understand the feedback:**
   - Read the comment body carefully
   - Note the file path and line range from the `path`, `line`, and `start_line` fields
   - Many review comments (especially from CodeRabbit) include a "Prompt for AI Agents" section with specific instructions

4. **Research the context:**
   - Read the relevant file(s) mentioned in the comment
   - Understand the surrounding code and its purpose
   - Check if the feedback is valid and applicable

5. **Implement the fix:**
   - Make the requested changes following the comment's guidance
   - Ensure changes are consistent with the codebase style and conventions

6. **Summarize:**
   - Briefly describe what changes were made to address the comment
