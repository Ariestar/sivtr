# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sivtr** is a terminal output workspace that captures, browses, searches, and reuses terminal command output and AI coding assistant sessions. Four AI providers (Codex, Claude Code, OpenCode, Pi) and four shells (Bash, Zsh, PowerShell, Nushell).

Architecture: CLI binary (`src/`) wrapping a core library (`crates/sivtr-core/`). Clap-based subcommands for copy, copy ref, search, show, work (sessions/records/parts), init, diff, hotkey, doctor. TUI mode for browse/search views.

## Development Commands

```bash
cargo build                                         # debug
cargo test --workspace                              # all tests
cargo fmt --all -- --check                          # format check
cargo clippy --workspace --all-targets -- -D warnings # clippy (strict)
```

Pre-commit gate:
```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Workspace Structure

```
crates/sivtr-core/src/     ← Core library (no CLI deps)
  ai.rs                    ← AgentProvider enum, session parsing
  claude.rs / codex.rs / opencode.rs / pi.rs  ← Per-provider parsers
  record/
    model.rs               ← WorkRecord, WorkPart, WorkTime (canonical model)
    refs.rs                ← WorkRef parsing (terminal/..., codex/..., pi/...)
    index.rs               ← Record indexing and lookup
  search/                  ← Search matcher and navigator
  workspace.rs             ← Workspace resolution (git root → sessions)
  config/                  ← SivtrConfig, serde TOML
  history/                 ← SQLite command history
  session.rs               ← Session log reading
  time.rs                  ← Timestamp normalization
src/                       ← CLI binary
  main.rs                  ← Command routing
  cli.rs                   ← All Clap definitions
  commands/
    init.rs                ← Shell hook injection + show/uninstall
    copy.rs                ← Copy + workspace picker
    search.rs              ← Search with pipeline chaining
    show.rs / diff.rs / work.rs  ← Ref display commands
    config.rs              ← Config init/show/edit
    doctor.rs              ← Environment diagnostics
    hotkey.rs              ← Global hotkey daemon (Windows)
    flush.rs               ← Internal: shell hook callback
  tui/                     ← Terminal UI framework
  command_blocks.rs        ← Command block parsing
```

## Key Data Types

- `WorkRecord` — single command execution or AI turn
- `WorkPart` / `WorkPartIo` — leaf content chunk; `WorkPartIo` is Input | Output, `WorkPartKind` is Prompt/Command/UserMessage/AssistantMessage/ToolCall/ToolOutput/Text/Error
- `WorkRef` — typed reference: `terminal/session_42/3/o/1`, `codex/abc123/5/i/2`, `pi/session/3/12`
- `WorkTime::from_components(started_at, ended_at, duration_ms)` — time construction
- `AgentProvider::Codex | Claude | OpenCode | Pi`

## Coding Rules

- **anyhow::Result** everywhere, always `.context("description")?`
- **No unwrap()** in production — tests use `expect("reason")`
- **No async** — single-threaded CLI
- **Workspace separation** — `sivtr-core` must not depend on CLI types
- **clippy strict** — `-D warnings` on CI
- **Rust 2021 edition, MSRV 1.88** — pinned in rust-toolchain.toml

## Working Directory

Always confirm before starting work:
```bash
pwd && git branch
```

## Shell Hook System

`sivtr init {shell}` injects precmd hooks using marker blocks (`# >>> sivtr shell integration >>>`). Legacy `sift` markers auto-migrated. Session logs go to `$XDG_STATE_HOME/sivtr/session_<pid>.log`. Internal `sivtr flush` called by hooks on each prompt.

## Search Pipeline

```bash
sivtr search terminal --status failure --json | sivtr search terminal --exclude "example" -f timeline
```

Target selectors: `terminal/<session>/<record>/<line>`, `agent/<session>/<turn>`, `<provider>/<session>/<turn>`. Part refs: `<provider>/<session>/<turn>/<i|o>/<part>`. Use `*` for wildcards.

## Diagnostics

```bash
sivtr doctor        # Check binary, config, session logs, hooks, providers, clipboard
sivtr init show     # Show which shell hooks are installed
sivtr init uninstall # Remove all shell hooks
```

Run `sivtr doctor` after any installation or when troubleshooting.
