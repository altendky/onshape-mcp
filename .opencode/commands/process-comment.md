---
description: Process a GitHub PR review comment
---

# Process GitHub PR Review Comment

**Comment reference:** $ARGUMENTS

## Process

1. **Validate and parse the input:**
   - If the input contains `/issues/` → **stop** and inform the user: "This appears to be an issue URL, not a PR review comment. Use `/process-issue` instead."
   - If the input contains `/pull/` but does NOT contain `#discussion_r`, `/files#r`, or `#r` → **stop** and inform the user: "This appears to be a PR URL without a comment reference. Use `/review-pr` to review the PR, or provide a specific comment URL (e.g., ending in `#discussion_r1234567890`)."
   - If given a URL like `https://github.com/owner/repo/pull/123#discussion_r1234567890` or `https://github.com/owner/repo/pull/123/files#r1234567890`, extract the comment ID (the number after `r`)
   - If given just a comment ID number, use it directly

2. **Fetch the comment and full thread:**
   - Determine the repository owner/repo from the URL or from `gh repo view --json owner,name`
   - Fetch the linked comment: `gh api repos/{owner}/{repo}/pulls/comments/{comment_id}`
   - Extract the PR number from the comment's `pull_request_url` field
   - Fetch all review comments for the PR: `gh api repos/{owner}/{repo}/pulls/{pull_number}/comments --paginate`
   - Build the thread:
     - If the linked comment has an `in_reply_to_id`, that value is the root comment's ID
     - If the linked comment has no `in_reply_to_id`, it is the root
     - Collect the root comment and all comments where `in_reply_to_id` equals the root's `id`
   - Display the full thread in chronological order using markdown separators:

     ```
     ---
     **@username** (2024-01-15 10:30 UTC): <- linked comment

     <comment body>

     ---
     **@another_user** (2024-01-15 11:45 UTC):

     <reply body>

     ---
     ```

   - Mark the specifically linked comment with `<- linked comment` so it's identifiable

3. **Verify branch:**
   - Get the PR's head branch using the PR number from step 2: `HEAD_BRANCH=$(gh pr view "$PR_NUMBER" --json headRefName --jq '.headRefName')`
   - Get current local branch: `CURRENT_BRANCH=$(git branch --show-current)`
   - Compare `HEAD_BRANCH` and `CURRENT_BRANCH`; if they don't match, inform the user and **stop**

4. **Understand the feedback:**
   - Read the entire thread to understand the full context of the discussion
   - Pay special attention to the specifically linked comment—it may indicate:
     - The most recent or relevant feedback to address
     - A specific decision or direction the user wants implemented
     - A follow-up request after earlier discussion
   - If the thread contains back-and-forth discussion, identify the current consensus or latest request
   - Note the file path and line range from the root comment's `path`, `line`, and `start_line` fields
   - Many review comments (especially from CodeRabbit) include a "Prompt for AI Agents" section—check all comments in the thread for such prompts

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
   - Show the changes made: run `git diff` to display all modifications
   - Ask if they are satisfied with the changes and would like to commit and push
   - If no, stop without committing (the changes remain in the working directory for manual review or further editing)

9. **Commit and push:**
   - Stage the changed files
   - Commit with message format:

     ```text
     review comment: <brief summary>

     <description of changes made to address the feedback>

     <comment-url>
     ```

   - Push to the remote branch

10. **Reply to the comment:**
    - Get the commit URL:
      - `COMMIT_SHA=$(git rev-parse HEAD)`
      - `REPO=$(gh repo view --json owner,name --jq '.owner.login + "/" + .name')`
      - Commit URL: `https://github.com/${REPO}/commit/${COMMIT_SHA}`
    - Draft a brief, conversational reply explaining what changes were made to address the feedback, including a link to the commit
    - Show the draft reply to the user and ask for confirmation before posting
    - If approved, post the reply:

      ```bash
      gh api repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies \
        -f body="<reply text>"
      ```

    - If not approved, skip posting (the commit is already pushed; user can reply manually if desired)
