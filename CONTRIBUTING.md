# Contributing to sivtr

Thanks for helping improve sivtr — local workspace memory for terminal output and AI coding sessions.

English · [简体中文](#中文)

## Ways to contribute

- **Bug reports** — unexpected CLI/TUI behavior, provider parsing gaps, install failures
- **Feature ideas** — open an issue first for non-trivial API or UX changes
- **Docs** — README, docs-site (`docs-site/`), skill text under `skills/`
- **Code** — Rust CLI (`src/`), core library (`crates/sivtr-core/`), optional VS Code bridge

Security issues: do not open a public issue. Prefer a private [GitHub Security Advisory](https://github.com/Ariestar/sivtr/security/advisories/new) or contact the maintainer via the email on the [GitHub profile](https://github.com/Ariestar).

## Development setup

Requirements:

- Rust stable (see `rust-toolchain.toml`; MSRV is pinned in `Cargo.toml` as `rust-version`)
- On Windows, a normal MSVC or GNU toolchain that can build the workspace

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
cargo build
cargo test --workspace
```

Optional docs site:

```bash
cd docs-site
bun install --frozen-lockfile
bun run build
```

## Local checks (same as CI)

PR CI runs on Windows, Ubuntu, and macOS (`.github/workflows/rust.yml`). `cargo audit` runs weekly / on demand. Before opening a PR:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Or the one-liner used as a pre-commit gate:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Project layout

```text
crates/sivtr-core/   core model, provider parsers, search, history, config
src/                 CLI commands, TUI, shell hooks, remote daemon, hotkey
docs-site/           Astro/Starlight documentation
editors/vscode/      VS Code bridge for the AI session picker
skills/              bundled agent skills (e.g. sivtr-memory)
changelogs/          archived per-release notes (pre-release-plz)
CHANGELOG.md         current changelog, updated by release-plz
```

Dependency direction: **CLI (`src/`) → `sivtr-core`**. Core must not import CLI/Clap types.

## Coding guidelines

- Prefer `anyhow::Result` with `.context("…")?` in production paths
- No `unwrap()` / `expect()` in non-test production code (tests may `expect("reason")`)
- Keep most CLI command handlers **blocking**; async stays inside the remote daemon
- Match surrounding style: naming, error messages, module layout (`execute()` entry, helpers, tests)
- See also `CLAUDE.md` and `.claude/rules/` if you use Claude Code in this repo

## Pull requests

1. Prefer a focused branch and a clear PR title (conventional style is welcome: `fix:`, `feat:`, `docs:`, …).
2. Describe **what** changed and **why**; link issues when relevant.
3. Include tests for behavioral fixes when practical.
4. Use a Conventional Commits PR title (`feat:`, `fix:`, `docs:`, …). Squash titles drive auto-release.
5. Do not bump `version` in `Cargo.toml` or add `changelogs/` in a feature PR — the release-plz Release PR does that after merge.

## Releases (maintainers)

[release-plz](https://release-plz.dev) drives versioning, changelog, and tagging (`release-plz.toml` + `.github/workflows/release-plz*.yml`). On every push to `main` it keeps a **Release PR** up to date: it bumps the workspace version, syncs the `sivtr-core` pin and `Cargo.lock`, and drafts a section in `CHANGELOG.md` from Conventional Commit squash titles. Merging that PR is the release.

- `feat:` → MINOR; `fix:` → PATCH; `feat!` → still MINOR in `0.x` (flagged as breaking); `chore` / `docs` / `ci` / … → no bump.
- Merge the Release PR immediately for a fresh release, or leave it open to batch several commits.
- Merging tags `vX.Y.Z`. That tag runs `.github/workflows/release.yml` (assets, crates.io publish for both crates, GitHub Release from the `CHANGELOG.md` section, installer smoke).
- release-plz pushes the Release PR with `GITHUB_TOKEN`, so CI does not run on it and required checks never turn green — merge it with the maintainer bypass (or add a workflow-capable PAT).

Install metadata for `cargo binstall` lives in `[package.metadata.binstall]` in the root `Cargo.toml`.

## Community

- Docs: https://sivtr.pages.dev/
- Issues: https://github.com/Ariestar/sivtr/issues
- Sponsorship: repository **Sponsor** button (`.github/FUNDING.yml`) → [WeChat tip page](https://sivtr.pages.dev/zh-cn/project/sponsor/)

---

## 中文

欢迎贡献 bug 报告、功能讨论、文档与代码。

### 本地开发

```bash
cargo build
cargo test --workspace
```

提交前与 CI 对齐：

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

### 约定摘要

- `src/`（CLI）依赖 `crates/sivtr-core/`，core 不得反向依赖 CLI
- 生产代码用 `anyhow` + `context`，避免 `unwrap`
- 安全问题请用私密渠道（Security Advisory / 维护者联系方式），不要公开 issue
- **不要**在普通 PR 里自行 bump `Cargo.toml` 版本；发版由 release-plz 的 Release PR 完成（`feat:` → MINOR、`fix:` → PATCH，合并即发版，tag 触发 `release.yml` 构建发布）

更细的模块与 Rust 约定见仓库内 `CLAUDE.md`、`.claude/rules/`。
