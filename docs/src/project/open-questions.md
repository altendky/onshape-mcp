# Open Questions

## Pending

- [ ] Documentation standards — Rustdoc coverage, README structure, usage examples
- [ ] Distribution details — cargo install setup, GitHub releases workflow, binary naming
- [x] npm wrapper package — ~~Evaluate publishing JS wrapper for `npx` installation~~ Resolved: using platform-specific optional deps pattern. See [npm Wrapper](npm-wrapper.md)
- [ ] Release process — Versioning strategy, changelog, release workflow, crates.io publishing
- [ ] Contribution guidelines — CONTRIBUTING.md, PR expectations, code review process
- [ ] Project files — Standard files not yet documented (.gitignore, .gitattributes, rustfmt.toml, clippy.toml, deny.toml, .editorconfig, codecov.yml, CHANGELOG.md, dependabot.yml, CODEOWNERS, issue/PR templates)
- [ ] Non-manual repository configuration — Discuss opportunities for automated repo config (Terraform, GitHub API, etc.)
- [ ] AI awareness setup — Create root-level AI context file (e.g., `AGENTS.md` or `CLAUDE.md`) that references REQUIREMENTS.md for project standards, architecture, and conventions; use symlinks for tool-specific locations (`.cursorrules`, `.github/copilot-instructions.md`, etc.) to maintain single source of truth; prioritize portable/standard formats over tool-specific syntax where possible
- [ ] AI file validation and testing — Review mechanisms for validating AI context files in CI (e.g., symlink integrity, reference validity, format linting)
- [ ] Git ignore strategy — Discuss root .gitignore vs distributed approach, required patterns (target/, IDE files, OS files, etc.)
- [x] Markdown validation — ~~Evaluate CI/pre-commit checks for markdown files~~ Implemented: markdownlint-cli2 for style/formatting, lychee for broken links. Remaining: prose linting with vale (deferred)

## Deferred

Items to address later in the project:

### CI/Infrastructure

- [ ] PR title enforcement — CI validation of PR title format (Conventional Commits or custom)
- [ ] Labels configuration — Standard labels for issues/PRs (bug, enhancement, etc.)

### Authentication Enhancements

- [ ] OAuth 2.0 authentication — Multi-user apps, team access (see [Authentication](authentication.md))

### Features

- [ ] FeatureScript support — Phase E tools (`onshape_eval_featurescript`, etc.)

### tracing-sansio Enhancements

- [ ] Independent publication — Extract to separate repository and publish to crates.io
- [ ] Async support — Add `capture_tracing_async()` if needed
- [ ] Event filtering — Add predicates to filter captured events

### Markdown Tooling

- [ ] Semantic newlines enforcement — Add `markdownlint-sentences-per-line` rule to enforce one sentence per line (requires Node.js/npm infrastructure)
