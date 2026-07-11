# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sivtr** is a terminal output workspace that captures, browses, searches, and reuses terminal command output and AI coding assistant sessions. Four AI providers (Codex, Claude Code, OpenCode, Pi, Hermes) and four shells (Bash, Zsh, PowerShell, Nushell). Cross-device remote memory uses a local daemon with Share/Grant/Mount over encrypted iroh transport.

Architecture: CLI binary (`src/`) wrapping a core library (`crates/sivtr-core/`). Clap-based subcommands for copy, search, show, work, filter, var, nav, zoom, init, diff, hotkey, doctor, serve, share, remote, peer, and workspace. TUI mode for browse/search views.

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
  claude.rs / codex.rs / opencode.rs / pi.rs / hermes.rs  ← Per-provider parsers
  record/
    model.rs               ← WorkRecord, WorkPart, WorkTime (canonical model)
    refs.rs                ← WorkRef parsing (local body + origin:body remote form)
    index.rs               ← Record indexing and lookup
  query/                   ← Workspace record/source loading
  search/                  ← Search matcher and navigator
  workspace.rs             ← Workspace resolution (git root → sessions), data_dir()
  config/                  ← SivtrConfig, serde TOML
  history/                 ← SQLite command history
  session.rs               ← Session log reading
  time.rs                  ← Timestamp normalization
src/                       ← CLI binary
  main.rs                  ← Command routing
  cli/
    mod.rs                 ← Top-level Clap definitions
    remote.rs              ← serve/share/peer/remote/workspace Clap types
  commands/
    capture/               ← copy, pipe, run, init, flush, import, diff, clear, browse
    memory/                ← search, filter, var, nav, zoom, show, work, workset
    remote/                ← serve, share, mounts, peer, workspace
    system/                ← config, doctor, history, hotkey, codex, migrate, version
  remote/                  ← Device daemon, identity, state, protocol, ipc
  tui/                     ← Terminal UI framework
```

## Key Data Types

- `WorkRecord` — single command execution or AI turn
- `WorkPart` / `WorkPartIo` — leaf content chunk; `WorkPartIo` is Input | Output, `WorkPartKind` is Prompt/Command/UserMessage/AssistantMessage/ToolCall/ToolOutput/Text/Error
- `WorkRef` — typed reference: local `terminal/session_42/3/o/1`, `codex/abc123/5/i/2`, or remote `desk:terminal/...` / `docs:codex/4` (`origin:body`)
- `WorkTime::from_components(started_at, ended_at, duration_ms)` — time construction
- `AgentProvider::Codex | Claude | OpenCode | Pi | Hermes`
- Remote model: **Device Daemon + Identity + Share + Grant + Mount**

## Coding Rules

- **anyhow::Result** everywhere, always `.context("description")?`
- **No unwrap()** in production — tests use `expect("reason")`
- **No async** in most CLI paths — remote daemon uses async internally; command handlers stay blocking
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

Target selectors: `terminal/<session>/<record>/<line>`, `agent/<session>/<turn>`, `<provider>/<session>/<turn>`. Part refs: `<provider>/<session>/<turn>/<i|o>/<part>`. Use `*` for wildcards. Remote origins: `desk:terminal/...`, `docs:codex/4`.

## Remote Memory

Device-scoped daemon auto-starts when share/remote commands need it.

```bash
sivtr share                   # interactive: pick workspace, print bare invite key
sivtr remote add desk <key>   # mount peer share under local alias
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr serve status            # daemon identity + share/peer counts
sivtr wb list                 # local workspace origin labels
```

State lives under `data_dir()` (`SIVTR_DATA_DIR` override, else platform config dir `/sivtr`): `identity.key`, `remote-state.db`, `daemon.json`, `daemon.lock`, `daemon.log`.

## Diagnostics

```bash
sivtr doctor        # Check binary, config, session logs, hooks, providers, clipboard
sivtr init show     # Show which shell hooks are installed
sivtr init uninstall # Remove all shell hooks
```

Run `sivtr doctor` after any installation or when troubleshooting.
