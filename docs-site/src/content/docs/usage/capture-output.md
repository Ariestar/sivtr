---
title: Capture Terminal Output
description: Use pipe mode, run mode, and shell session import.
---

Capture is the first step in turning terminal output into reusable text. Use the lightest capture path that matches what you need.

## Choose a capture path

| Use case | Best command | Keeps command metadata? |
| --- | --- | --- |
| Inspect one existing command pipeline | `command 2>&1 \| sivtr` | No |
| Let `sivtr` run one command | `sivtr run command` | Partially, for the captured run |
| Open the current shell's recorded work | `sivtr import` | Yes, after shell integration |
| Copy one recent command block | `sivtr copy out` | Yes, after shell integration |
| Search saved output history | `sivtr history search "query"` | Yes, for saved captures |

## Pipe mode

Pipe mode reads stdin, writes it to history (when enabled), and opens the result in the external editor.

```bash
ls -la | sivtr
cargo build 2>&1 | sivtr
rg "TODO" . | sivtr
```

Use pipe mode when:

- the command already exists in your shell history;
- you want normal shell behavior for pipelines and redirection;
- you do not need `sivtr` to know the original command.

For commands that write important output to stderr, redirect stderr to stdout:

```bash
cargo test 2>&1 | sivtr
```

## Run mode

Run mode executes the command through `sivtr`:

```bash
sivtr run cargo test
sivtr run git status --short
```

Use run mode when:

- you want `sivtr` to execute and capture a single command;
- you want the exit status reported and the output saved before the editor opens;
- you prefer not to manage shell redirection manually.

Run mode captures stdout and stderr together and saves the result to history. If the command produces no output, `sivtr` exits after reporting that nothing was captured.

## Shell session import

Shell integration records structured command entries over time. After installing it, open the current session log:

```bash
sivtr import
```

This is useful when you have been working normally and later want to open the accumulated session in the editor.

Install shell integration with:

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

Restart the shell after installation.

## History capture

Captured output is saved to local history when `[history].auto_save` is enabled. Search it later with:

```bash
sivtr history search "panic"
sivtr history show 42
```

History is separate from the current shell session log: history is a longer-lived SQLite store, while session logs are per-shell structured records for recent command blocks.
