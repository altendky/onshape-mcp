# Open Questions

## Pending

- [ ] Documentation standards — Rustdoc coverage, README structure, usage examples
- [ ] Distribution details — cargo install setup, GitHub releases workflow, binary naming
- [x] npm wrapper package — ~~Evaluate publishing JS wrapper for `npx` installation~~ Resolved: using platform-specific optional deps pattern. See [npm Wrapper](npm-wrapper.md)
- [ ] Release process — Design in progress, see [Release](release.md#open-items) for current state and remaining decisions
- [ ] Contribution guidelines — CONTRIBUTING.md, PR expectations, code review process
- [ ] Project files — Standard files not yet documented (.gitignore, .gitattributes, rustfmt.toml, clippy.toml, deny.toml, .editorconfig, codecov.yml, CHANGELOG.md, CODEOWNERS, issue/PR templates). Note: dependabot.yml is now configured, see [CI > Dependency Monitoring](ci.md#dependency-monitoring)
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

- [ ] HMAC-SHA256 request signing — Per-request signatures with nonce and timestamp for replay protection; secret key never sent over the wire (see [Authentication](authentication.md) and [Onshape API key docs](https://onshape-public.github.io/docs/auth/apikeys/#request-signature))
- [x] OAuth 2.0 authentication — ~~Multi-user apps, team access~~ Implemented: authorization code flow via OpenCode plugin, token file storage, `AuthMethod::OAuth` variant. Token refresh deferred to future `onshape-client-io` crate. See [Authentication](authentication.md)
- [ ] OAuth token refresh — Automatic token refresh when expired (requires HTTP client in onshape-client-io crate)
- [ ] Standalone OAuth flow — Built-in browser/callback flow in the MCP binary itself (currently only via OpenCode plugin)
- [ ] OS keyring integration — Store tokens in system keychain instead of file (macOS Keychain, Windows Credential Manager, Linux Secret Service)

### Features

- [ ] FeatureScript support — Phase E tools (`onshape_eval_featurescript`, etc.)

### Tracing Crate Naming

The tracing capture crate needs a name for publication to crates.io.
The crate captures `tracing` spans and events **locally as return values** instead of routing them through the standard global subscriber.
This avoids global state and I/O side effects, enabling deterministic testing of pure/sans-IO code that emits tracing instrumentation.

Candidates:

- [ ] **`tracing-pure`** — "Pure" in the functional programming sense: no side effects. Accurate on two levels — the crate itself is pure (captures to memory, no I/O), and it preserves purity in code that uses it. The crate's own purity is the mechanism that makes it work: if the tracing layer performed I/O, the code under test would no longer be sans-IO. Simple and immediately understood by anyone familiar with the concept. The term "pure" is somewhat overloaded in Rust ("pure Rust" can mean no FFI/unsafe), but in the context of a crate description the intended meaning should be clear.
- [ ] **`tracing-reify`** — "Reify" means to make something abstract or implicit into something concrete. Trace events that would normally be implicit side effects flowing to global state are instead reified into data structures you can inspect and assert on. Technically precise about the core transformation the crate performs — effects become values. Memorable, but requires knowing (or looking up) the word.
- [ ] **`tracing-local`** — Directly contrasts with how `tracing` normally works: global subscriber vs local capture. Immediately communicates that trace capture is scoped to a call site rather than process-wide. The simplest and most approachable option. Downside: "local" describes the mechanism (where capture happens) rather than the motivation (preserving purity / avoiding I/O).
- [ ] **`tracing-sans-io`** — Says exactly what it is: tracing support for the sans-IO architectural pattern. No ambiguity about purpose for anyone familiar with the term. Downside: "sans-IO" has minimal adoption in Rust crate names (~40k total downloads across all crates using the term). Ties the crate's identity to a specific architectural label rather than describing what it does, which limits appeal to people who use the same pattern but don't use the term.

Both `tracing-pure` and `tracing-reify` are available on crates.io.
The companion proc-macro crate would follow the same naming pattern (e.g., `tracing-pure-macros`).

### Tracing Crate Enhancements

- [ ] Independent publication — Extract to separate repository and publish to crates.io
- [ ] Async support — Add `capture_tracing_async()` if needed
- [ ] Event filtering — Add predicates to filter captured events

### Markdown Tooling

- [ ] Semantic newlines enforcement — Add `markdownlint-sentences-per-line` rule to enforce one sentence per line (requires Node.js/npm infrastructure)
