---
title: Browse and Select
description: Navigate the workspace browser and single-buffer browser, select text, and copy.
---

`sivtr` has two interactive surfaces:

- the **workspace browser** (bare `sivtr` on a TTY, or `sivtr copy --pick` / hotkey): multi-source Source → Sessions → Dialogues → Content;
- the **single-buffer browser** (piped stdin or `sivtr run` / `sivtr pipe`): one captured output buffer.

## Open the workspace browser

```bash
sivtr                     # TTY: multi-source workspace browser
sivtr --all               # also select remote mounts on open
sivtr copy --pick         # same browser, for copy
sivtr copy claude --pick
```

Layout: Source · Sessions · Dialogues · Content. Content splits into **Input** and **Output** halves with independent scroll.

### Workspace navigation

| Key | Action |
| --- | --- |
| `0` / `1` / `2` / `3` | Focus Source, Sessions, Dialogues, or Content |
| `h` / `l` | Previous / next pane |
| `j` / `k` | Move down / up |
| `Space` | Toggle selection (source, session, or dialogue) |
| `a` | Select all sources (Source) · toggle all dialogues (Dialogues) |
| `g` / `t` | Select agent sources / terminal source (Source) |
| `R` | Refresh next level under active rows |
| `v` | Range-select dialogues · visual text select on Content |
| `Tab` | Switch Content Input ↔ Output half |
| `r` | Toggle fold/full content (structure markers vs expanded payloads) |
| `Ctrl-d` / `Ctrl-u` · `PgDn` / `PgUp` | Scroll content |
| `g` / `G` | Content top / bottom |
| `i` / `o` / `y` / `c` | Copy input / output / block / command |
| `Enter` | Confirm / open next / copy |
| `/` | Search |
| `z` | Toggle focused pane fullscreen |
| `t` | Open Vim-style full view (Sessions/Dialogues) |
| `?` | Help |
| `q` / `Esc` | Quit / back |

Mouse: drag on Content selects text; `Ctrl`-drag is block select. Source list expands when focused; unfocused stays a compact strip. Content half heights bias toward the focused half.

Structure parts (tool / skill / thinking) show as `<:channel:…:>` markers in fold mode; `r` expands full payloads.

See [Keybindings](/reference/keybindings/) for the full table.

## Single-buffer browser

Pipe capture or `sivtr run` opens a Vim-shaped read-only browser for one buffer.

| Key | Action |
| --- | --- |
| `j` / `k` · arrows | Move |
| `Ctrl-D` / `Ctrl-U` | Half page |
| `Ctrl-F` / `Ctrl-B` · Page keys | Page |
| `gg` / `G` | Top / bottom |
| `/` · `n` / `N` | Search · next / previous match |
| `v` / `V` / `Ctrl-V` | Character / line / block select |
| `y` | Copy selection |
| mouse drag · `Ctrl`-drag | Select · block select |
| `e` | Open selection (or whole buffer) in the configured editor |
| `[[` / `]]` | Previous / next command block (session logs) |
| `myy` / `myi` / `myo` / `myc` | Copy block / input / output / bare command |

Configure the editor:

```toml
[editor]
command = "nvim"
```
