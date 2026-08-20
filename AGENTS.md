# AI Agent Instructions

This file is the single source of truth for AI coding assistants working on this project. Tool-specific instruction files, such as `CLAUDE.md`, should point here instead of duplicating these rules.

Keep this file limited to durable, cross-session guidance. Current progress, temporary state, and session scratch do not belong here.

## Project

- Purpose:
- Stack:
- Package manager:

## Commands

- Install:
- Dev:
- Test:
- Check:

## Communication

- Use Simplified Chinese for user-facing explanations, questions, progress updates, and summaries.
- Keep code, identifiers, comments, logs, test names, and commit messages in English.
- Match the surrounding language when editing existing documentation.

## Working Agreement

- Stay within the stated scope. Do not add, refactor, or improve unrelated functionality.
- Preserve unrelated working-tree changes and never discard user work.
- Follow the project's existing structure, conventions, and patterns.
- Adding, updating, or removing dependencies is allowed and encouraged when it benefits the project. Always propose the change first and wait for the user's explicit approval before applying it.
- If a request is ambiguous, risky, or has materially different approaches, explain the trade-offs and ask before changing files.
- If the safest path is clear and within scope, proceed and state any important assumption.
- Treat a request containing `dry-run` as read-only. Explain what would be done without modifying files or configuration.

## Engineering Principles

- Make the smallest precise change that solves the problem; avoid unrelated refactoring.
- Identify business invariants and express each invariant in one authoritative place.
- Centralize shared validation, configuration, authorization, caching, and API contracts.
- Do not use broad `try/catch` blocks to swallow errors.
- Do not hide failures behind silent fallbacks.
- Prefer readable business flow over fragmented helper methods.
- Do not split a cohesive workflow into many one-use private methods.
- Introduce abstractions only for real reuse, a clear reduction in complexity, or an excessively long method.
- Use the Unix toolchain integrated into the terminal when it is the best fit for the task.

## Tool Routing

- Use CodeGraph for structural code questions: symbol definitions and signatures, callers and callees, execution flow, and change impact.
- Use `rg` for literal text, comments, log messages, and filenames.
- The terminal provides the Unix command set via **uutils-coreutils** (installed through Scoop `main` bucket at `D:\Workspace\Apps\Scoop\apps\uutils-coreutils\current\`). Use Unix commands when they are the best fit for the task. Note: in PowerShell, `ls`, `cat`, `cp`, `mv`, `rm` etc. are shadowed by aliases to native cmdlets (`Get-ChildItem`, `Get-Content`, ...) — invoke the Unix binaries by full path, or use the PowerShell cmdlets directly.
- Prefer integrated CodeGraph MCP tools. If `.codegraph/` is missing, ask before running `codegraph init -i`.
- Use Sivtr before asking the user to repeat terminal output, prior decisions, validation evidence, debugging history, or earlier agent context.
- Search Sivtr narrowly and expand only the relevant records. Treat retrieved memory as evidence, then verify current files or commands before making claims about current state.
- Use `gh` for all GitHub API and pull-request access (`gh api`, `gh pr`, `gh issue`, ...). `gh` authenticates automatically from the `GH_TOKEN`/`GITHUB_TOKEN` user environment variable; never pass tokens on the command line.
- Authentication lives in exactly one place: the `GH_TOKEN`/`GITHUB_TOKEN` user environment variable. Do not add duplicate auth paths or fallbacks (netrc entries, hardcoded tokens, per-shell wrapper functions).
- Never hit `api.github.com` with raw `curl` or `Invoke-RestMethod`: unauthenticated calls share a low anonymous rate limit and fail with HTTP 403. Route through `gh api` instead.

## Verification

- Run the relevant project checks after making changes.
- Fix failures caused by the current change, then rerun the checks.
- If a relevant check cannot be run, state the reason explicitly.
- Review the final diff and ensure it contains only task-related changes.

## Git Workflow

- Inspect the working tree before creating a commit. Never include unrelated or unrecognized dirty files.
- Create commits only when the user explicitly asks. Committing, pushing, or opening a pull request is never part of finishing a task: "done" means files changed, checks run, and the diff summarized, with the working tree left uncommitted. Group each commit around one coherent change.
- Commit messages must follow Conventional Commits (full format in the [Conventional Commits](#conventional-commits) section).
- Do not push unless the user explicitly requests it.
- Never delete remote branches, including merged feature branches.
- Never force-push or rewrite shared branch history.
- `main` accepts squash merges only. PR titles are Conventional Commits and become the squash subject.

### Per-change workflow

- Before any set of edits goes into commits, cut a working branch from `main` (`git checkout -b <type>/<short-topic>`). Commit directly on `main` only when the change is a one-off fix that will not become a PR.
- Split the work into one commit per coherent unit (feature / refactor / docs / chore), never one big mixed commit, and never commit unrelated changes together.
- Group related units that belong to the same module or feature area into one branch. When the user explicitly asks, open one draft PR per branch. A PR's title is a Conventional Commit and becomes the squash subject on merge.

## Conventional Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):

```text
<type>[scope][!]: <short summary>

[body]

[footer(s)]
```

- **type** — lowercase: `feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `ci`, `perf`, `build`, `revert`.
- **scope** (optional) — affected area in parentheses, e.g. `feat(parser):`.
- **`!`** — breaking change, e.g. `feat!:`. Equivalent: `BREAKING CHANGE:` in the footer.
- **summary** — imperative, lowercase, ≤ 50 chars: `fix crash on empty input`.
- **footer** (optional) — `BREAKING CHANGE: <desc>`, `Fixes #123`, `Refs #456`.

Example:

```text
feat(api)!: require auth token on all endpoints

BREAKING CHANGE: unauthenticated requests are now rejected with 401.
```

Version-bump mapping per type lives in [Version Management](#version-management).

## Pull Request Workflow

- Open or update a pull request only when the user explicitly asks.
- Use a dedicated branch (`<type>/<short-topic>`, see [Branch discipline](#branch-discipline)) and follow any repository-specific branch naming convention. Do not perform feature work directly on the default branch.
- When one branch depends on another unmerged branch, stack the PRs with `gh stack init <bottom> ... <top>` then `gh stack submit --auto` (draft PRs; `--open` only when the user asks for ready-for-review); each PR's base points at its dependency so diffs stay minimal until the base merges. `gh stack sync` keeps the stack in sync after upstream merges.
- Before opening a pull request, inspect the working tree, commits, and complete diff against the intended base branch. Remove unrelated changes from the pull request scope.
- Run the relevant checks before opening the pull request. Open every pull request as a draft and keep it a draft until the user explicitly asks to mark it ready for review — pushing commits or opening a non-draft pull request triggers GitHub's automated AI review, so do not trigger it before the user asks for review.
- Follow the repository's existing pull request template. Do not replace or bypass project-specific requirements.
- Write the pull request title in Conventional Commits style.
- Keep the pull request body factual and concise. Include the purpose, main changes, validation actually performed, related issue when known, and any material risks or follow-up work.
- After review feedback, address comments within scope, rerun affected checks, and summarize the resolution. Do not silently introduce unrelated changes.
- Do not merge, enable auto-merge, close, reopen, or change the base branch unless the user explicitly requests it.
- Keep the remote source branch after merge.

## Version Management

How this project versions and releases changes. AI agents must follow this when preparing or performing releases.

### Version numbers (SemVer)

- Versions are `MAJOR.MINOR.PATCH` per SemVer. In the `0.x` phase: MINOR carries new user-visible features (it may include breaking changes); PATCH is reserved for backwards-compatible bug fixes and hotfixes; MAJOR stays unused until the API/config surface is stable enough to commit to `1.0.0`.
- Pre-release suffixes (`-alpha.N`, `-beta.N`, `-rc.N`) mark unstable builds. Keep them few and converge quickly; do not let a release candidate drag on.

### Release flow

[release-plz](https://release-plz.dev) drives versioning, changelog, and tagging (`release-plz.toml` + `.github/workflows/release-plz*.yml`). On every push to `main` it keeps a **Release PR** up to date: it bumps `[workspace.package]` version, syncs the `sivtr-core` pin and `Cargo.lock`, and drafts a Keep a Changelog section in `CHANGELOG.md` from Conventional Commit squash titles. Merging that PR is the release.

1. `feat:` → MINOR (`0.4.1` → `0.5.0`); `fix:` → PATCH (`0.4.1` → `0.4.2`); `feat!` / `BREAKING CHANGE` → still MINOR in `0.x`, flagged as breaking in the changelog; `chore` / `docs` / `ci` / `refactor` / `test` / `perf` / `build` → no version bump.
   - Only `feat`/`fix` commits open a Release PR (`release_commits` in `release-plz.toml`); non-versioned work rides along with the next release instead of shipping its own.
   - semver-check runs only on `sivtr-core` (the published library). `sivtr` is a binary; its internal API changes never inflate the version — breaking changes there are signaled by `feat!` / `BREAKING CHANGE` commits.
2. **Immediately** = merge the Release PR right after the feature lands. **Batch** = leave it open and let later squashes accumulate in it.
3. Edit the changelog section inside the Release PR before merging if a hand-written note is wanted; release-plz never overwrites existing release sections.
4. Only `sivtr` is tagged (`vX.Y.Z`) and gets a changelog entry; `sivtr-core` shares `version.workspace`, its version and the `sivtr-core` pin are synced by the same Release PR, and both crates are published by the tag pipeline. A change touching only `crates/sivtr-core/` bumps core but creates no tag — it ships with the next `sivtr` release.
5. Merging the Release PR creates tag `vX.Y.Z`. The tag runs `.github/workflows/release.yml` (assets, crates.io publish for both crates, GitHub Release from the `CHANGELOG.md` section, installer smoke). Do not produce or attach artifacts manually.

Merge gate for the Release PR: release-plz pushes its branch with `GITHUB_TOKEN`, so GitHub does not run CI on it and the required status checks never go green — only the maintainer bypass can merge it. (Optional: swap in a workflow-capable PAT secret to restore CI on the Release PR.)

The tag step (`release-plz-release.yml`) pushes the `vX.Y.Z` tag with a PAT: the `RELEASE_PLZ_TOKEN` secret is assigned to the action's `GITHUB_TOKEN` env. `GITHUB_TOKEN` pushes do not trigger other workflows, so the PAT is required for the tag push to fire `release.yml`. Only `RELEASE_PLZ_TOKEN` must be set — the action reads the git token from `GITHUB_TOKEN`, not from a separate env.

### Branch discipline

- One branch per task; merge back to main within 2–3 days. No long-lived parallel branches.
- Branch naming: `<type>/<short-topic>` (e.g. `feat/search-index-cache`, `fix/daemon-start`), lowercased with dashes. Do not use a user's name, dates, or arbitrary numbers.
- Delete branches and prune worktrees immediately after merging. Never leave worktree checkouts behind.
- main must always be releasable; green CI is the merge gate. No feature work directly on main.

### Commits and changelog

- Conventional Commits (see [Conventional Commits](#conventional-commits) for the full syntax) drive version bumps. Squash titles become the changelog entries; keep them imperative and lowercase.
- Do not bump `version` in `Cargo.toml` or add `changelogs/` in a feature PR — the Release PR does that.
- New release notes go into `CHANGELOG.md`; old per-release files under `changelogs/` are archive only.

### Common traps

- Hoarding changes into one big release → merge the Release PR per feature batch instead.
- PATCH creep (many patches within one MINOR) → it is time for a new MINOR; PATCH is for hotfixes.
- Branch pile-up (unmerged branches, abandoned worktrees) → merge and clean up.
- Hotfix not merged back into main → the bug returns in the next release.
- Version mismatch between tag and manifests → the release pipeline fails; release-plz bumps all manifests together (`Cargo.toml`, `sivtr-core` pin, `Cargo.lock`).

## Project Invariants

- Add project-specific constraints here.

## Durable Handoff

- Record durable architecture, contracts, workflows, known limitations, and recurring gotchas in the appropriate project documentation.
- Do not turn durable documentation into a session journal.
- Keep any documentation touched during the task accurate within its scope.

## Done

- Relevant checks pass, or any unrun checks are explained.
- The diff contains only task-related changes.
- The completion summary states what changed, key decisions, verification performed, and any remaining risks.
