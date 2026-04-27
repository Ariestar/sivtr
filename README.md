# sivtr

**Terminal Output Workspace** - Capture, browse, search, select, and export terminal output.

`sivtr` turns terminal output into searchable, selectable, reusable text assets. It is not a terminal emulator or multiplexer - it is an independent tool that works alongside your existing terminal workflow.

## Features (V1)

- **Pipe mode**: `command | sivtr` - pipe any command output into a TUI browser
- **Run mode**: `sivtr run <command>` - wrap command execution, capture output, then browse
- **Vim-style navigation**: `hjkl`, `Ctrl-D/U`, `gg`, `G`
- **Modal workflow**: normal / insert / visual / search
- **Visual selection**: `v` (character), `V` (line), `Ctrl-V` (block/column)
- **Search**: `/pattern` forward search, `n`/`N` for next/previous match
- **Copy to clipboard**: `y` in visual mode copies selection to system clipboard
- **History**: local SQLite storage with full-text search via FTS5
- **Codex capture**: read structured Codex rollout logs and copy conversation blocks without opening a TUI
- **Cross-platform**: Windows, macOS, Linux

## Installation

```bash
cargo install --path .
```

## Usage

```bash
# Pipe mode
cargo build 2>&1 | sivtr
ls -la | sivtr

# Run mode
sivtr run cargo test
sivtr run python script.py

# History
sivtr history list
sivtr history search "error"
sivtr history show 42

# Import scrollback (tmux/zellij, coming soon)
sivtr import

# Codex conversation capture
sivtr copy codex
sivtr copy codex out
sivtr copy codex in
sivtr copy codex tool
sivtr copy codex all --print

# Global hotkey (Windows)
sivtr hotkey start
sivtr hotkey status
sivtr hotkey stop
```

`sivtr copy codex` reads Codex rollout JSONL files from `~/.codex/sessions` and defaults to the
latest session whose `cwd` matches your current working directory. The common zero-confirmation
paths are:

- `sivtr copy codex` copies the last completed user + assistant turn
- `sivtr copy codex out` copies the last assistant reply
- `sivtr copy codex in` copies the last user message
- `sivtr copy codex tool` copies the last tool output

Selector semantics match `sivtr copy`: `1` means the latest matching item, `2` means the
2nd-latest, and ranges like `2..4` select multiple recent items.

Progress commentary emitted while Codex is working is filtered out by default, so `copy out`
returns the final assistant reply instead of intermediate status updates.

On Windows, `sivtr hotkey start` registers a single global shortcut. By default it uses
`alt+y` and opens a new terminal window that runs `sivtr copy codex --pick`.

## Key Bindings

| Key | Mode | Action |
|-----|------|--------|
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
| `y` | Visual | Yank (copy) to clipboard |
| `Esc` | Visual/Search/Insert | Cancel |
| `q` | Normal | Quit |

## Tech Stack

- **Language**: Rust
- **CLI**: clap
- **TUI**: ratatui + crossterm
- **Storage**: SQLite (rusqlite) + FTS5
- **Clipboard**: arboard (cross-platform)

## Project Structure

```text
sivtr/
|- crates/sivtr-core/    # Core library (capture, parse, buffer, selection, search, history, export)
|- src/                  # CLI + TUI binary
|  |- cli.rs             # Command definitions (clap)
|  |- app.rs             # Application state machine
|  |- tui/               # TUI rendering and events
|  `- commands/          # Subcommand handlers
`- docs/                 # PRD and architecture docs
```

## License

MIT
