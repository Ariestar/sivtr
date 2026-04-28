---
title: sivtr
description: Terminal output workspace for the AI era.
---

`sivtr` turns terminal output into reusable text. Pipe output into a browser, wrap command execution, search across past captures, copy structured command blocks, or pull the last useful turn out of a Codex session without opening the transcript by hand.

It is not a terminal emulator, multiplexer, or replacement shell. It works beside the terminal you already use.

Source code and releases live at [github.com/Ariestar/sivtr](https://github.com/Ariestar/sivtr).

## Why it exists

Terminal output is usually treated as disposable. Once a command scrolls away, the useful part is trapped in scrollback, copy mode, or a giant log. `sivtr` gives that output a small workspace:

- capture output from stdin, subprocesses, or shell integration;
- browse output in a Vim-style TUI;
- select character, line, or block ranges;
- copy recent command blocks by semantic selector;
- search saved output with SQLite FTS5;
- reuse Codex conversation blocks from the current project.

## First command

Install from crates.io, then pipe output into `sivtr`:

```bash
cargo install sivtr
cargo test 2>&1 | sivtr
```

Inside the browser, use `j` and `k` to move, `/` to search, `v` or `V` to select, `y` to copy, and `q` to quit.

## Common workflows

| Goal | Command |
| --- | --- |
| Browse command output | `cargo test 2>&1 \| sivtr` |
| Run and capture a command | `sivtr run cargo test` |
| Open the current session log | `sivtr import` |
| Copy the latest command output | `sivtr copy out` |
| Pick one or more recent blocks | `sivtr copy --pick` |
| Copy the latest assistant reply from Codex | `sivtr copy codex out` |
| Search saved captures | `sivtr history search "panic"` |
| Start the Windows Codex hotkey | `sivtr hotkey start` |

## Documentation map

- Start with [Installation](/start/installation/) and [Quickstart](/start/quickstart/).
- Learn the mental model in [Core Concepts](/start/core-concepts/).
- Use task pages under [Use sivtr](/usage/capture-output/).
- Keep exact syntax open in [CLI Reference](/reference/cli/).
- Read the implementation shape in [Architecture](/explanation/architecture/).
