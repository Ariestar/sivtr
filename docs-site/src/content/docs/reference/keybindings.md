---
title: Keybindings
description: TUI, picker, and Vim-view keybindings.
---

## Browser

| Key | Mode | Action |
| --- | --- | --- |
| `j` / `Down` | Normal, Insert, Visual | Move down |
| `k` / `Up` | Normal, Insert, Visual | Move up |
| `h` / `Left` | Normal, Insert, Visual | Move left |
| `l` / `Right` | Normal, Insert, Visual | Move right |
| `0` / `Home` | Normal, Insert, Visual | Start of line |
| `^` | Normal, Insert, Visual | First non-blank |
| `$` / `End` | Normal, Insert, Visual | End of line |
| `Ctrl-D` | Normal, Insert, Visual | Half page down |
| `Ctrl-U` | Normal, Insert, Visual | Half page up |
| `Ctrl-F` / `PageDown` | Normal, Insert, Visual | Page down |
| `Ctrl-B` / `PageUp` | Normal, Insert, Visual | Page up |
| `gg` | Normal, Visual | Top |
| `G` | Normal, Visual | Bottom |
| `H` | Normal, Visual | Top of viewport |
| `M` | Normal, Visual | Middle of viewport |
| `L` | Normal, Visual | Bottom of viewport |
| `/` | Normal, Insert | Search |
| `n` | Normal | Next match |
| `N` | Normal | Previous match |
| `i` | Normal | Insert mode |
| `v` | Normal, Visual | Character selection |
| `V` | Normal, Visual | Line selection |
| `Ctrl-V` | Normal, Visual | Block selection |
| `o` | Visual | Swap selection anchor |
| `y` | Visual | Copy selection |
| `e` | Normal, Visual | Open editor |
| `Esc` | Insert, Visual, Search | Cancel current mode |
| `q` | Normal | Quit |

## Command-block navigation

Only available when the buffer has parsed command blocks.

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

## Copy picker

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down |
| `k` / `Up` | Move up |
| `Space` | Toggle current entry |
| `v` | Mark range anchor |
| `a` | Toggle all |
| `p` | Toggle preview |
| `t` | Open Vim-style full view |
| `Enter` | Confirm |
| `Esc` | Cancel |

## Vim-style full view

This view is opened from the picker with `t`.

| Key | Action |
| --- | --- |
| `[[` | Previous block |
| `]]` | Next block |
| `myy` | Copy block |
| `myi` | Copy input |
| `myo` | Copy output |
| `myc` | Copy command |
| `mvv` | Select block |
| `mvi` | Select input |
| `mvo` | Select output |
| `T` | Toggle alternate tool view when available |
| `p`, `q`, `Esc` | Return to picker |
