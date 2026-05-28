<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  Local workspace memory for terminal output and AI coding sessions.
  <br>
  Capture what happened, search it later, and let agents reuse exact local evidence.
</p>

<p align="center">
  <a href="https://crates.io/crates/sivtr"><img alt="Crates.io" src="https://img.shields.io/crates/v/sivtr?style=flat-square"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ariestar.sivtr-vscode"><img alt="VS Code Marketplace" src="https://vsmarketplacebadges.dev/version/ariestar.sivtr-vscode.svg?style=flat-square&label=VS%20Code&color=007ACC"></a>
  <a href="https://github.com/Ariestar/sivtr/actions/workflows/rust.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/Ariestar/sivtr/rust.yml?branch=main&style=flat-square"></a>
  <a href="https://deepwiki.com/Ariestar/sivtr"><img alt="Ask DeepWiki" src="https://deepwiki.com/badge.svg?repo=Ariestar/sivtr"></a>
  <a href="rust-toolchain.toml"><img alt="Rust" src="https://img.shields.io/badge/rust-1.88%2B-orange?style=flat-square"></a>
  <a href="https://linux.do/"><img alt="linux.do" src="https://img.shields.io/badge/friend-linux.do-1f883d?style=flat-square"></a>
</p>

<p align="center">
  <strong>English</strong>
  ·
  <a href="README.zh-CN.md">简体中文</a>
  ·
  <a href="https://sivtr.pages.dev/">Docs</a>
  ·
  <a href="https://sivtr.pages.dev/zh-cn/">中文文档</a>
</p>

<p align="center">
  <a href="https://sivtr.pages.dev/playbooks/fix-terminal-error/">
    <img src="docs-site/public/demo/1.gif" alt="sivtr demo: find and reuse recent terminal output" width="820">
  </a>
</p>

---

## Why sivtr?

Developers and agents lose time reconstructing context that already exists locally: terminal failures, test output, tool logs, and previous AI sessions. `sivtr` turns that work into searchable memory.

With `sivtr`, you can:

- stop pasting logs into agents by hand;
- search terminal and AI sessions from the current workspace;
- copy or print exact records, lines, and input/output parts;
- save result sets and pass them through command chains.

> [!IMPORTANT]
> For agent workflows, install both the `sivtr` CLI and the bundled `sivtr-memory` skill. The CLI stores and retrieves local memory; the skill teaches your agent when and how to use it.

## Features

- **Terminal capture** for Bash, Zsh, PowerShell, and Nushell.
- **Fast TUI** for browsing long command output.
- **Unified search** across terminal records, Codex, Claude Code, OpenCode, and Pi sessions.
- **Stable refs** for exact records, lines, and input/output parts.
- **WorkSets** for reusable result sets: `@last`, saved `@name`, and stdin `@`.
- **Chainable commands**: search, narrow, expand, and show evidence without copying intermediate refs.
- **Agent-ready memory** through the bundled `sivtr-memory` skill.
- **Diagnostics** with `sivtr doctor`, `sivtr init show`, and `sivtr init uninstall`.

## Quick start

Install the CLI:

```bash
cargo install sivtr
```

Or use the prebuilt installer on Linux/macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

Enable shell capture:

```bash
sivtr init bash       # or zsh, powershell, nushell
sivtr doctor
```

Capture and browse output:

```bash
cargo test 2>&1 | sivtr
```

Search recent workspace memory:

```bash
sivtr s agent -m "TODO|decision|failed" --since today -f timeline
sivtr s terminal --status failure --latest 1 --refs
```

## Agent memory

Install the bundled skill globally:

```bash
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

Then ask your coding agent to use local memory first:

```text
Fix the latest terminal error. Use sivtr first.
```

Instead of asking you to paste logs, the agent can search local evidence, inspect exact refs, patch the code, and verify the fix.

## Examples

### Copy recent terminal output

```bash
sivtr copy out --print
sivtr copy cmd --pick
```

### Search, save, and show exact evidence

```bash
sivtr s terminal -m "panic|failed" --save failures --refs
sivtr work parts @failures --io output --refs
sivtr show @last --full
```

### Chain memory commands

```bash
sivtr s terminal -m "panic|failed" \
  | sivtr work parts @ --io output \
  | sivtr s @ -m "error|stack|caused by" \
  | sivtr show @ --full
```

### Build a recent work timeline

```bash
sivtr s agent --since today --sort oldest -f timeline
sivtr s terminal --since today --sort oldest -f timeline
```

### Expand context around search hits

```bash
sivtr s agent -m "release|validation" --save release_context --refs
sivtr zoom @release_context -C 2 --save expanded_context
sivtr show @expanded_context --full
```

## Core concepts

| Concept | Meaning |
| --- | --- |
| WorkRecord | One useful work event: a terminal command, agent turn, tool call, or captured output block. |
| WorkPart | A typed input/output piece inside a record, such as command, assistant reply, tool output, or error. |
| WorkRef | A stable address for a record, line, or part, for example `pi/<session>/3/o/1`. |
| WorkSet | An ordered result set that can be saved, selected, piped, expanded, and shown. |

Common handles:

| Handle | Use |
| --- | --- |
| `@last` | Most recent WorkSet. |
| `@name` | WorkSet saved with `--save name`. |
| `@name[1,3..5]` | 1-based selection from a saved WorkSet. |
| `@` | WorkSet JSON received from stdin. |

## Command overview

| Command | Purpose |
| --- | --- |
| `sivtr` / `sivtr pipe` | Read stdin and open the output browser. |
| `sivtr run <command>` | Execute a command, capture output, then browse it. |
| `sivtr copy` | Copy recent terminal command blocks. |
| `sivtr copy <provider>` | Copy content from Codex, Claude Code, OpenCode, or Pi sessions. |
| `sivtr search` / `sivtr s` | Search terminal and agent memory; saves results as `@last`. |
| `sivtr work sessions` | List terminal and agent sessions in the current workspace. |
| `sivtr work records <source>` | Project a source or WorkSet to record refs. |
| `sivtr work parts <source>` | Project records to canonical input/output part refs. |
| `sivtr show <ref-or-workset>` | Print exact refs or WorkSets. |
| `sivtr zoom <source>` | Add neighboring records around anchors. |
| `sivtr diff <left> <right>` | Compare recent command blocks. |
| `sivtr doctor` | Diagnose binary, config, session logs, hooks, providers, and clipboard. |
| `sivtr init <shell>` | Install shell integration; also supports `show` and `uninstall`. |
| `sivtr config` | Manage the TOML config file. |
| `sivtr history` | List, search, and show captured output history. |
| `sivtr hotkey` | Manage the Windows AI session picker hotkey daemon. |

## Supported sources

| Source | Support |
| --- | --- |
| Terminal | Bash, Zsh, PowerShell, Nushell shell hooks; pipe and run capture. |
| Codex | Local rollout/session JSONL files. |
| Claude Code | Local transcript/session files. |
| OpenCode | Local session data. |
| Pi | Local Pi agent session logs. |

## Documentation

- Documentation: [https://sivtr.pages.dev/](https://sivtr.pages.dev/)
- 中文文档: [https://sivtr.pages.dev/zh-cn/](https://sivtr.pages.dev/zh-cn/)
- Playbooks: [https://sivtr.pages.dev/playbooks/](https://sivtr.pages.dev/playbooks/)
- CLI reference: [docs-site/src/content/docs/reference/cli.md](docs-site/src/content/docs/reference/cli.md)
- Memory skill: [skills/sivtr-memory](skills/sivtr-memory)

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Docs site:

```bash
cd docs-site
bun install --frozen-lockfile
bun run build
```

Repository layout:

```text
crates/sivtr-core/  core model, provider parsers, search, history, config
src/                CLI commands, TUI, shell hooks, hotkey integration
docs-site/          Astro/Starlight documentation site
editors/vscode/     VS Code bridge for the AI session picker
skills/             bundled agent skills
```
