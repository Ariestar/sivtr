---
title: Quickstart
description: Capture output, search it, select text, and copy command blocks.
---

This quickstart covers the main workflow: capture terminal output, browse it, copy a selected range, and then reuse recent command blocks.

## 1. Pipe output into the browser

```bash
cargo test 2>&1 | sivtr
```

The TUI opens with the combined output. Use:

- `j` / `k` to move line by line;
- `Ctrl-D` / `Ctrl-U` to move by half pages;
- `gg` / `G` to jump to the top or bottom;
- `/panic` to search;
- `n` / `N` for next or previous match;
- `q` to quit.

## 2. Select and copy

In the browser:

1. Move to the start of the text.
2. Press `V` for line-wise selection.
3. Move to the end of the range.
4. Press `y`.

The selected text is copied to the system clipboard.

For a rectangular selection, press `Ctrl-V` instead of `V`.

## 3. Wrap a command

Use `sivtr run` when you want `sivtr` to execute the command and capture stdout/stderr:

```bash
sivtr run cargo test
sivtr run python scripts/check.py
```

`sivtr` prints the process exit status, then opens the captured output.

## 4. Enable command-block copy

Install shell integration once:

```bash
sivtr init powershell
```

Restart the shell, run a few commands, then copy recent blocks:

```bash
sivtr copy
sivtr copy out
sivtr copy cmd 2
sivtr copy 2..4 --print
```

Selectors are relative to the newest block. `1` means the latest command block, `2` means the one before it, and `2..4` means a recent range.

## 5. Copy Codex output

From inside a project directory with Codex sessions:

```bash
sivtr copy codex out
sivtr copy codex in
sivtr copy codex tool --regex error
sivtr copy codex all --lines 1:40
```

`sivtr` finds the newest Codex session whose working directory matches the current project.
