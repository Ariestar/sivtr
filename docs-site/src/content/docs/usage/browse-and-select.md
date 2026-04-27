---
title: Browse and Select
description: Navigate the TUI, search output, select text, and open an editor.
---

The built-in browser is read-only and Vim-shaped. It is designed for scanning large terminal output and extracting the useful part.

## Navigation

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down |
| `k` / `Up` | Move up |
| `h` / `Left` | Move left |
| `l` / `Right` | Move right |
| `0` / `Home` | Start of line |
| `^` | First non-blank column |
| `$` / `End` | End of line |
| `Ctrl-D` | Half page down |
| `Ctrl-U` | Half page up |
| `Ctrl-F` / `PageDown` | Page down |
| `Ctrl-B` / `PageUp` | Page up |
| `gg` | Top |
| `G` | Bottom |
| `H` / `M` / `L` | Top, middle, or bottom of view |

## Search

Press `/`, type a pattern, and press `Enter`.

| Key | Action |
| --- | --- |
| `/` | Start search |
| `Enter` | Run search |
| `Esc` | Cancel search input |
| `n` | Next match |
| `N` | Previous match |

Search jumps to matching rows and shows match count in the status line.

## Selection

| Key | Action |
| --- | --- |
| `v` | Character selection |
| `V` | Line selection |
| `Ctrl-V` | Block selection |
| `o` | Swap selection anchor |
| `y` | Copy selection to clipboard |
| `Esc` | Cancel selection |

Mouse selection is also supported. Drag with the left mouse button to start a selection. Hold `Ctrl` while dragging for block mode.

## Command-block shortcuts

When browsing a structured session log, `sivtr` can jump, copy, or select the current command block.

| Key | Action |
| --- | --- |
| `[[` | Previous command block |
| `]]` | Next command block |
| `myy` | Copy current command block |
| `myi` | Copy current command input |
| `myo` | Copy current command output |
| `myc` | Copy current bare command |
| `mvv` | Select current command block |
| `mvi` | Select current command input |
| `mvo` | Select current command output |

## Editor handoff

Press `e` to open the current selection, or the whole buffer if nothing is selected, in the configured editor.

Configure the editor:

```toml
[editor]
command = "nvim"
```
