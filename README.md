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
```

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
