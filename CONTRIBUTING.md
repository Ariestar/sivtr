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

CI runs on Windows, Ubuntu, and macOS (`.github/workflows/rust.yml`). Before opening a PR:

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
changelogs/          per-release notes
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
4. Update docs or `changelogs/` only when the change is user-facing and you are preparing a release — day-to-day PRs need not bump the crate version.
5. Do not bump `version` in `Cargo.toml` unless the maintainer asked for a release PR.

Maintainer will handle versioning, tags, and crates.io / GitHub Releases.

## Releases (maintainers)

Release assets and crates publish are driven by `.github/workflows/release.yml` from a version tag (e.g. `v0.2.6`). Install metadata for `cargo binstall` lives in `[package.metadata.binstall]` in the root `Cargo.toml`.

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
- **不要**在普通 PR 里自行 bump `Cargo.toml` 版本；发版由维护者处理

更细的模块与 Rust 约定见仓库内 `CLAUDE.md`、`.claude/rules/`。
