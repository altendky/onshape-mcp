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

3. **Verify branch:**
   - Extract the PR number from the comment's `pull_request_url` field
   - Get the PR's head branch: `gh pr view <N> --json headRefName`
   - Compare with current local branch: `git branch --show-current`
   - If they don't match, inform the user and **stop**

4. **Understand the feedback:**
   - Read the comment body carefully
   - Note the file path and line range from the `path`, `line`, and `start_line` fields
   - Many review comments (especially from CodeRabbit) include a "Prompt for AI Agents" section with specific instructions

5. **Research the context:**
   - Read the relevant file(s) mentioned in the comment
   - Understand the surrounding code and its purpose
   - Check if the feedback is valid and applicable

6. **Implement the fix:**
   - Make the requested changes following the comment's guidance
   - Ensure changes are consistent with the codebase style and conventions

7. **Summarize:**
   - Briefly describe what changes were made to address the comment

8. **Confirm with user:**
   - Ask if they are satisfied with the changes and would like to commit and push
   - If no, stop

9. **Commit and push:**
   - Stage the changed files
   - Commit with message format:

     ```text
     review comment: <brief summary>

     <description of changes made to address the feedback>

     <comment-url>
     ```

   - Push to the remote branch
