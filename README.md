<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  Terminal output workspace for the AI era.
  <br>
  Capture, sift, browse, search, select, and reuse terminal output and Codex sessions.
</p>

<p align="center">
  <a href="https://crates.io/crates/sivtr"><img alt="Crates.io" src="https://img.shields.io/crates/v/sivtr?style=flat-square"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ariestar.sivtr-vscode"><img alt="VS Code Marketplace" src="https://vsmarketplacebadges.dev/version/ariestar.sivtr-vscode.svg?style=flat-square&label=VS%20Code&color=007ACC"></a>
  <a href="https://github.com/Ariestar/sivtr/actions/workflows/rust.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/Ariestar/sivtr/rust.yml?branch=main&style=flat-square"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square"></a>
  <a href="rust-toolchain.toml"><img alt="Rust" src="https://img.shields.io/badge/rust-1.88%2B-orange?style=flat-square"></a>
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

---

## What is sivtr?

`sivtr` turns noisy terminal streams into reusable text assets. It is built for developers who move between shells, build logs, test failures, AI-agent replies, tool output, and long Codex sessions.

It is not a terminal emulator and not a multiplexer. It is a companion tool for the terminal workflows you already use.

## Highlights

- Browse command output in a fast keyboard-first TUI.
- Pipe any command into a searchable, selectable output viewer.
- Record shell command blocks and copy recent inputs, outputs, or bare commands.
- Read Codex session JSONL files and copy useful user, assistant, or tool blocks.
- Open a Codex picker from VS Code with one shortcut.
- Filter copied text with regex and line ranges.
- Keep a local SQLite history for later search.
- Compare recent command outputs while iterating on tests and builds.

## Install

Install the CLI from crates.io:

```bash
cargo install sivtr
```

Install from source:

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
cargo install --path .
```

Install the VS Code bridge from the Marketplace:

```text
ariestar.sivtr-vscode
```

The extension launches the Codex picker from the current workspace. If the `sivtr` CLI is missing, it offers to run `cargo install sivtr` in a visible terminal.

## Quick Start

Browse command output:

```bash
cargo test 2>&1 | sivtr
```

Run a command through `sivtr` and inspect the captured output:

```bash
sivtr run cargo build
```

Copy the latest command block from the current shell session:

```bash
sivtr copy
```

Copy the latest assistant reply from the current Codex project session:

```bash
sivtr copy codex out
```

Open an interactive picker for Codex conversation blocks:

```bash
sivtr copy codex --pick
```

Compare two recent command outputs:

```bash
sivtr diff 1 2
```

## Core Workflows

### Browse Output

Use pipe mode when you already have a command:

```bash
some-command --verbose 2>&1 | sivtr
```

Use run mode when you want `sivtr` to execute, capture, and then open output:

```bash
sivtr run cargo test
```

Inside the TUI, move with Vim-style keys, search with `/`, enter visual selection with `v`, and copy with `y`.

### Copy Command Blocks

With shell integration enabled, `sivtr` records command blocks so you can copy recent inputs and outputs later:

```bash
sivtr copy              # latest input + output
sivtr copy out          # latest output only
sivtr copy in 2..4      # user input from recent blocks
sivtr copy cmd --pick   # pick and copy bare commands
```

Selectors are newest-first: `1` is the latest block, `2` is the one before it, and `2..4` selects multiple blocks.

Filters run after text is assembled:

```bash
sivtr copy out --regex panic
sivtr copy out --lines 10:40
```

### Reuse Codex Sessions

`sivtr copy codex` reads Codex rollout JSONL files from `~/.codex/sessions`. When you run it inside an active `codex` or `codex resume` shell, it prefers that exact session id first. Otherwise it chooses the newest session whose `cwd` matches your current directory. Use `--session N` to target the Nth newest recorded session, or `--session ID` to target a specific session id / id prefix.

```bash
sivtr copy codex        # latest completed user + assistant turn
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy codex out    # latest assistant reply
sivtr copy codex out --session 2 --print
sivtr copy codex in     # latest user message
sivtr copy codex tool   # latest tool output
sivtr copy codex all    # parsed session
sivtr copy codex --session 2 --pick
sivtr copy codex all --max-blocks 0
sivtr copy codex all --max-blocks 10000
```

Progress commentary is filtered by default, so `sivtr copy codex out` returns the final assistant reply instead of intermediate status updates.

Large Codex transcripts are capped to the latest `10000` parsed blocks by default for robustness. Set `[codex].max_blocks = 0` in config or pass `--max-blocks 0` for a full import.

To mirror sessions for another local account (for example read-only sharing),
run export watch mode from the source account:

```bash
sivtr codex export --dest /srv/sivtr/root-codex --watch --interval-ms 500
```

`--watch` defaults to a 1-second sync interval. Use `--interval-ms` for
sub-second updates when you need faster session visibility.

### VS Code Shortcut

The VS Code extension contributes:

```text
Sivtr: Pick Codex Turn
```

Default keybinding:

```text
Alt+Y
```

You can rebind it to `Ctrl+Y`, but that usually overrides the editor Redo shortcut.

On Linux, this VS Code shortcut works as the default picker shortcut when the
editor has focus. The extension runs:

```bash
sivtr hotkey-pick-agent --cwd . --provider all
```

and `sivtr` prefers the active `codex` / `codex resume` session when one is
available.

On macOS, the same VS Code shortcut works as the default picker shortcut when
the editor has focus.

### Linux Shortcut Setup

Linux does not currently ship a default global `sivtr` hotkey outside VS Code.

Reasons:

- Wayland does not provide a universal cross-desktop global hotkey API for
  ordinary CLI apps.
- X11-only approaches are legacy and do not cover common Wayland desktops.
- Opening the picker also needs an interactive terminal, and Linux does not
  have one portable terminal-launch command that works across GNOME, KDE,
  Sway, headless SSH, and tmux-based Codex setups.

Recommended Linux setups:

- VS Code: use the built-in `Alt+Y` command binding.
- tmux: bind a key to the current pane's working directory:

```tmux
bind-key y new-window -c "#{pane_current_path}" "sivtr copy codex --pick"
```

- Terminal / desktop environment: create a custom shortcut that launches
  `cd <project-path> && sivtr copy codex --pick` in a terminal for the project
  you want to inspect.

### macOS Shortcut Setup

macOS does not currently ship a built-in `sivtr` global hotkey daemon. The
recommended default is the VS Code shortcut above.

For a project-local Terminal launcher plus a LaunchAgent wrapper, generate the
helper files on macOS:

```bash
sivtr init macos-shortcut
```

This writes:

- `~/.local/bin/sivtr-pick-codex`
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`

You can:

- run `~/.local/bin/sivtr-pick-codex` directly;
- load the LaunchAgent with `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`;
- keep using the VS Code command for the most reliable shortcut-driven flow.

### Windows Global Hotkey

On Windows, the hotkey daemon can open the Codex picker from anywhere:

```bash
sivtr hotkey start
sivtr hotkey status
sivtr hotkey stop
```

The default shortcut is `alt+y`.

## Command Reference

| Command | Purpose |
| --- | --- |
| `sivtr` / `sivtr pipe` | Read output from stdin and open the TUI browser. |
| `sivtr run <command>` | Execute a command, capture output, then browse it. |
| `sivtr copy` | Copy recent command blocks. |
| `sivtr copy codex` | Copy useful content from the current Codex session. |
| `sivtr diff <left> <right>` | Compare recent command blocks. |
| `sivtr history` | List, search, and show captured output history. |
| `sivtr config` | Manage the TOML config file. |
| `sivtr init <shell>` | Generate shell integration for command-block capture. |
| `sivtr import` | Open the current session log. |
| `sivtr hotkey` | Manage the Windows Codex picker hotkey. |
| `sivtr clear` | Clear session logs. |

## TUI Keys

| Key | Mode | Action |
| --- | --- | --- |
| `j` / `Down` | Normal | Move down |
| `k` / `Up` | Normal | Move up |
| `h` / `Left` | Normal | Move left |
| `l` / `Right` | Normal | Move right |
| `Ctrl-D` | Normal | Half page down |
| `Ctrl-U` | Normal | Half page up |
| `g` | Normal | Go to top |
| `G` | Normal | Go to bottom |
| `/` | Normal | Start search |
| `n` / `N` | Normal | Next / previous match |
| `v` / `V` / `Ctrl-V` | Normal | Visual, visual line, visual block |
| `y` | Visual | Copy selection to clipboard |
| `Esc` | Visual/Search/Insert | Cancel |
| `q` | Normal | Quit |

## Documentation

- English docs: [https://sivtr.pages.dev/](https://sivtr.pages.dev/)
- Chinese docs: [https://sivtr.pages.dev/zh-cn/](https://sivtr.pages.dev/zh-cn/)
- VS Code extension: [editors/vscode/README.md](editors/vscode/README.md)

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

VS Code extension:

```bash
cd editors/vscode
pnpm install
pnpm run compile
pnpm run package
```

Workspace layout:

```text
sivtr/
|- crates/sivtr-core/    # Capture, parsing, buffers, selection, search, history, export
|- src/                  # CLI, TUI, commands, hotkey integration
|- docs-site/            # Astro/Starlight documentation site
|- editors/vscode/       # VS Code extension bridge for the Codex picker
`- .github/workflows/    # CI and release automation
```

## License

sivtr is licensed under the [Apache License 2.0](LICENSE).
