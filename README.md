# sivtr

[![Crates.io](https://img.shields.io/crates/v/sivtr.svg)](https://crates.io/crates/sivtr)
[![CI](https://github.com/Ariestar/sivtr/actions/workflows/rust.yml/badge.svg)](https://github.com/Ariestar/sivtr/actions/workflows/rust.yml)
[![License](https://img.shields.io/crates/l/sivtr.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](rust-toolchain.toml)

**Terminal output workspace for the AI era** - Capture, sift, browse, search, select, and reuse your terminal output and AI coding sessions.

`sivtr` turns command output, Codex sessions, command blocks, and tool results into searchable, selectable, reusable text assets. It is not a terminal emulator or multiplexer. It is a companion tool for the terminal workflows you already use.

## Why sivtr

Modern coding work happens in streams: shell output, test failures, build logs, AI-agent replies, tool calls, and long terminal histories. `sivtr` gives those streams a workspace.

- Pipe any output into a fast TUI browser.
- Wrap commands and keep their output for later.
- Search, select, and copy exactly the text you need.
- Reuse recent command blocks without digging through scrollback.
- Pull the useful parts of the current Codex session without opening raw transcript files.
- Compare recent command outputs when iterations get noisy.

## Installation

```bash
cargo install sivtr
```

From source:

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
cargo install --path .
```

## Quick Start

Browse command output:

```bash
cargo test 2>&1 | sivtr
```

Run a command through `sivtr` and inspect the captured output:

```bash
sivtr run cargo build
```

Copy the last command block from the current shell session:

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

Use run mode when you want `sivtr` to execute, capture, and then open the output:

```bash
sivtr run cargo test
```

Inside the TUI you can move with Vim-style keys, search with `/`, enter visual selection with `v`, and copy with `y`.

### Copy Command Blocks

With shell integration enabled, `sivtr` records command blocks so you can copy recent inputs and outputs later:

```bash
sivtr copy              # latest input + output
sivtr copy out          # latest output only
sivtr copy in 2..4      # user input from recent blocks
sivtr copy cmd --pick   # pick and copy bare commands
```

Selectors are relative to newest first: `1` is the latest block, `2` is the one before it, and ranges like `2..4` select multiple blocks.

Filters run after text is assembled:

```bash
sivtr copy out --regex panic
sivtr copy out --lines 10:40
```

### Reuse Codex Sessions

`sivtr copy codex` reads Codex rollout JSONL files from `~/.codex/sessions` and chooses the newest session whose `cwd` matches your current directory.

```bash
sivtr copy codex        # latest completed user + assistant turn
sivtr copy codex out    # latest assistant reply
sivtr copy codex in     # latest user message
sivtr copy codex tool   # latest tool output
sivtr copy codex all    # parsed session
```

Progress commentary is filtered by default, so `sivtr copy codex out` returns the final assistant reply instead of intermediate status updates.

On Windows, the hotkey daemon opens the Codex picker from anywhere:

```bash
sivtr hotkey start
sivtr hotkey status
sivtr hotkey stop
```

The default shortcut is `alt+y`.

### Search History

Captured output is stored locally and can be searched later:

```bash
sivtr history list
sivtr history search "error"
sivtr history show 42
```

### Configure

Create, inspect, or edit the config file:

```bash
sivtr config init
sivtr config show
sivtr config edit
```

Generate shell integration hooks:

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

## Commands

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
| `i` | Normal | Enter insert mode |
| `v` | Normal | Enter visual mode |
| `V` | Normal | Enter visual line mode |
| `Ctrl-V` | Normal | Enter visual block mode |
| `/` | Normal | Start search |
| `n` | Normal | Next search match |
| `N` | Normal | Previous search match |
| `y` | Visual | Copy selection to clipboard |
| `Esc` | Visual/Search/Insert | Cancel |
| `q` | Normal | Quit |

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workspace contains:

```text
sivtr/
|- crates/sivtr-core/    # Capture, parsing, buffers, selection, search, history, export
|- src/                  # CLI, TUI, commands, hotkey integration
|- docs-site/            # Astro/Starlight documentation site
|- editors/vscode/       # VS Code extension bridge for the Codex picker
`- .github/workflows/    # CI and release automation
```

## License

MIT
