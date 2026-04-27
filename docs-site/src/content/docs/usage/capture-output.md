---
title: Capture Output
description: Use pipe mode, run mode, and session logs.
---

## Pipe mode

Pipe mode reads stdin and opens the result.

```bash
ls -la | sivtr
cargo build 2>&1 | sivtr
rg "TODO" . | sivtr
```

Use pipe mode when:

- the command already exists in your shell history;
- you want normal shell behavior for pipelines and redirection;
- you do not need `sivtr` to know the original command.

For commands that write important output to stderr, redirect it:

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

- you want `sivtr` to capture the command directly;
- you want the exit status printed before browsing;
- you prefer not to manage shell redirection manually.

Run mode captures combined output. If the command produces no output, `sivtr` exits after reporting that nothing was captured.

## Session import

After shell integration is installed, `sivtr import` opens the current session log:

```bash
sivtr import
```

This is useful when you have been working normally and later want to browse the accumulated session as a single workspace.

## Choosing the capture path

| Use case | Best command |
| --- | --- |
| Inspect one command's output | `command 2>&1 \| sivtr` |
| Run a command through the tool | `sivtr run command` |
| Browse everything recorded in this shell | `sivtr import` |
| Copy one recent command block without opening TUI | `sivtr copy out` |
| Search saved captures | `sivtr history search "query"` |
