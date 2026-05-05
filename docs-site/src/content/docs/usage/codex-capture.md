---
title: Codex Capture
description: Copy useful blocks from the current Codex session.
---

`sivtr copy codex` reads Codex rollout JSONL files from `~/.codex/sessions`. By default it chooses the newest session whose `cwd` matches the current working directory.

This is useful when you want to reuse the last answer, input, tool output, or whole parsed session without opening the Codex transcript manually.

## Defaults

```bash
sivtr copy codex
```

The default copies the last completed user plus assistant turn.

## Copy a specific kind

```bash
sivtr copy codex out
sivtr copy codex in
sivtr copy codex tool
sivtr copy codex all
```

| Command | Copies |
| --- | --- |
| `sivtr copy codex` | Last user plus assistant turn |
| `sivtr copy codex out` | Last assistant reply |
| `sivtr copy codex in` | Last user message |
| `sivtr copy codex tool` | Last tool output |
| `sivtr copy codex all` | Whole parsed session |

## Select older items

Selectors work the same way as command-block copy:

```bash
sivtr copy codex 2
sivtr copy codex 2..4
sivtr copy codex out 3
```

`1` means the newest matching Codex unit, `2` means the second newest, and so on.

## Filter Codex text

```bash
sivtr copy codex tool --regex error
sivtr copy codex all --lines 1:40
```

Use `--print` to inspect the copied text:

```bash
sivtr copy codex out --print
```

## Pick interactively

```bash
sivtr copy codex --pick
sivtr copy codex out --pick
```

The plain CLI picker starts with the session list, then lets you choose one or more units from that session. Press `t` to open the Vim-style view. In Codex views, `T` toggles tool content when an alternate full view is available.

Context-aware launchers such as the Windows hotkey and VS Code extension first open the newest non-empty session for the current workspace. If that session is missing or empty, they fall back to the session list.

## Windows hotkey

On Windows, the hotkey daemon opens the Codex picker for the project directory where it was started:

```bash
sivtr hotkey start
```

The default chord is `alt+y`. Configure it in `[hotkey]` or pass `--chord` when starting.
