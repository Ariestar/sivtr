<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  Local workspace memory for terminal output and AI coding sessions.
  <br>
  Capture what happened, search it later, and let agents reuse exact local evidence.
  <br>
  <strong>Your agent memory doesn’t need to be a heavyweight knowledge system.</strong>
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
  <a href="https://sivtr.pages.dev/playbooks/">
    <img src="docs-site/public/demo/4.gif" alt="sivtr demo: save search results as variables and keep narrowing" width="820">
  </a>
  <br>
  <sub>
    Save matches as memory variables and keep narrowing ·
    <a href="https://sivtr.pages.dev/playbooks/fix-terminal-error/">Fix terminal errors</a> ·
    <a href="https://sivtr.pages.dev/playbooks/recent-work-timeline/">Build timelines</a> ·
    <a href="https://sivtr.pages.dev/playbooks/agent-handoff/">Handoff with evidence</a>
  </sub>
</p>

---

## Why sivtr?

Developers and agents lose time reconstructing context that already exists locally: terminal failures, test output, tool logs, and previous AI sessions. `sivtr` turns that work into searchable memory without asking you to adopt a heavyweight knowledge system.

With `sivtr`, you can:

- ask an agent to fix the latest failure without pasting the log;
- find yesterday's test output, build error, or decision in seconds;
- reopen the exact command output or agent reply behind a summary;
- save useful search results as named variables like `@failures` and reuse them in the next command.

> [!IMPORTANT]
> For agent workflows, install both the `sivtr` CLI and the bundled `sivtr-memory` skill. The CLI stores and retrieves local memory; the skill teaches your agent when and how to use it.

## Features

- **Shell history that keeps the output**: capture commands from Bash, Zsh, PowerShell, and Nushell, including stdout, stderr, exit code, cwd, and timing.
- **A viewer for long output**: pipe `cargo test`, build logs, or stack traces into a fast keyboard-first TUI.
- **One search box for local work**: search terminal output and Codex, Claude Code, Hermes, OpenCode, and Pi sessions from the current repo.
- **Click-back / copy-back evidence**: every match can be shown, copied, expanded with nearby context, or handed to an agent.
- **Named memory variables**: save any result set as `@failures`, reuse `@last`, pass stdin as `@`, list vars with `sivtr var list`, and select slices like `@failures[1,3..5]`.
- **Deterministic anchor navigation**: move refs through parent/child/sibling/session structure with `sivtr nav`, without implicit expansion.
- **Agent-ready memory** through the bundled `sivtr-memory` skill.
- **Cross-device access**: expose a workspace read-only and browse another device's sessions with a `desk:...` ref, like reading local — for collaborative dev.
- **Diagnostics** with `sivtr doctor`, `sivtr init show`, and `sivtr init uninstall`.

## Quick start

Install the prebuilt CLI (no Rust toolchain needed):

```bash
cargo binstall sivtr
```

Other ways:

```bash
brew install ariestar/sivtr/sivtr   # macOS/Linux via Homebrew
cargo install sivtr                  # build from source (needs Rust)
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh   # Linux/macOS/WSL one-liner
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Ariestar/sivtr/main/install.ps1 | iex
```

Enable shell capture:

```bash
sivtr init bash       # or zsh, powershell, nushell
sivtr doctor
```

> [!NOTE]
> On Windows, if `sivtr init powershell` reports that the profile did not load, raise the current-user execution policy once with `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. sivtr never edits the registry — the hook lives only in your PowerShell profile.

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

Instead of asking you to paste logs, the agent can search local evidence, open the exact matching output, patch the code, and verify the fix.

## Examples

More end-to-end walkthroughs live in the [Playbooks](https://sivtr.pages.dev/playbooks/).

| Workflow | What you do | Demo |
| --- | --- | --- |
| Fix the latest terminal error | Ask your agent: <br><code>Fix the latest terminal error. Use sivtr first.</code> | <img src="docs-site/public/demo/1.gif" alt="Fix the latest terminal error with sivtr" width="320"> |
| Browse and copy recent terminal output | <code>cargo test 2&gt;&amp;1 &#124; sivtr</code><br><code>sivtr copy out --print</code> | <img src="docs-site/public/demo/2.gif" alt="Browse and copy recent terminal output" width="320"> |
| Turn recent work into a timeline | <code>sivtr s agent --since today --sort oldest -f timeline</code><br><code>sivtr s terminal --since today --sort oldest -f timeline</code> | <img src="docs-site/public/demo/3.gif" alt="Build a recent work timeline" width="320"> |
| Save results as variables and chain them | <code>sivtr s terminal -m "panic" --save failures</code><br><code>sivtr filter @failures --status failure --refs</code><br><code>sivtr var list</code> | <img src="docs-site/public/demo/4.gif" alt="Chain saved memory variables" width="320"> |
| Continue after interruption | Ask your agent: <br><code>Continue. Use sivtr memory first.</code> | <img src="docs-site/public/demo/5.gif" alt="Continue after interruption with sivtr memory" width="320"> |
| Prepare a handoff for the next agent | Ask your agent: <br><code>Give the next agent a handoff with evidence.</code> | <img src="docs-site/public/demo/6.gif" alt="Prepare an evidence-backed handoff" width="320"> |

## Core concepts

| Concept | Meaning |
| --- | --- |
| WorkRecord | One useful work event: a terminal command, agent turn, tool call, or captured output block. |
| WorkPart | The command, output, assistant reply, tool output, or error inside a record. Use this when you only need the useful part, not the whole event. |
| WorkRef | A stable address for one exact piece of memory, for example `pi/<session>/3/o/1`. Good for citations and reproducible handoffs. |
| WorkSet | The data behind memory variables such as `@last` and `@failures`: an ordered list of refs you can filter, save, slice, pipe, navigate, expand, and show. |

Memory variables:

| Handle | Use |
| --- | --- |
| `@last` | The latest search or projection result. |
| `@name` | A named variable created with `--save name` or `sivtr var set name`, for example `@failures`. |
| `@name[1,3..5]` | Pick only a few items from a saved variable. |
| `@` | Use the result coming from the previous command in a pipeline. |

## Command overview

| Command | Purpose |
| --- | --- |
| `sivtr` / `sivtr pipe` | Read stdin and open the output browser. |
| `sivtr run <command>` | Execute a command, capture output, then browse it. |
| `sivtr copy` | Copy recent terminal command blocks. |
| `sivtr copy <provider>` | Copy content from Codex, Claude Code, Hermes, OpenCode, or Pi sessions. |
| `sivtr search` / `sivtr s` | Search terminal and agent memory; saves matches as `@last`. |
| `sivtr filter <source>` | Apply the shared WorkSet filters to a source or piped WorkSet. |
| `sivtr var` | List, save, remove, merge, drop, or clean up named WorkSet variables. |
| `sivtr nav <source> <motion>` | Move anchors deterministically with `<`, `>N`, `+N`, `-N`, `[A..B]`, and `~`. |
| `sivtr work sessions` | List terminal and agent sessions in the current workspace. |
| `sivtr work records <source>` | Turn sessions or saved variables into event-level refs. |
| `sivtr work parts <source>` | Extract only useful inputs/outputs from matching events. |
| `sivtr show <ref-or-workset>` | Print the content behind refs, `@last`, `@name`, or piped results. Also accepts remote refs like `desk:terminal/...`. |
| `sivtr zoom <source>` | Add surrounding record context around search hits. |
| `sivtr diff <left> <right>` | Compare recent command blocks. |
| `sivtr serve` | Start/stop the local remote-memory daemon. |
| `sivtr share` | Explicitly share a local workspace for remote peers. |
| `sivtr remote` | Mount remote shares into the current workspace (`add`/`list`/`remove`/`test`). |
| `sivtr workspace` / `sivtr wb` | List known local workspaces (origin labels for `name:body` refs). |
| `sivtr doctor` | Diagnose binary, config, session logs, hooks, providers, and clipboard. |
| `sivtr init <shell>` | Install shell integration; also supports `show` and `uninstall`. |
| `sivtr config` | Manage the TOML config file. |
| `sivtr history` | List, search, and show captured output history. |
| `sivtr hotkey` | Manage the Windows AI session picker hotkey daemon. |

## Remote access

Two devices running sivtr can read each other's workspace sessions like reading local — for collaborative work where you want to see a teammate's terminal output or AI session without leaving your machine.

Refs use a single form: `origin:body`.

```text
codex/4                 # local current workspace
docs:codex/4            # another local workspace by name
desk:terminal/...       # mounted remote alias
alice/sivtr:hermes/...  # device/workspace coordinate
```

On the device that owns the workspace:

```bash
sivtr share                   # pick workspace (Enter = current), print invite key
sivtr wb list                 # see local workspace origin labels
```

On the other device:

```bash
sivtr remote add desk <invite-key>   # bare key from `sivtr share` stdout
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr show desk:terminal/session_42/3/o/1
sivtr zoom desk:terminal/session_42/3 -C 2
sivtr nav desk:terminal/session_42/3 +1 --refs
sivtr copy ref desk:terminal/session_42/3/o/1 --print
```

Sharing is opt-in and read-only. Secrets are redacted by default before data leaves the machine. Remote access uses encrypted iroh transport; the daemon auto-starts when needed. Unregistered origins error — register mounts with `sivtr remote add`, or list local workspaces with `sivtr wb`.

## Supported sources

| Source | Support |
| --- | --- |
| Terminal | Bash, Zsh, PowerShell, Nushell shell hooks; pipe and run capture. |
| Codex | Local rollout/session JSONL files. |
| Claude Code | Local transcript/session files. |
| Hermes | Local Hermes session JSONL files. |
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
