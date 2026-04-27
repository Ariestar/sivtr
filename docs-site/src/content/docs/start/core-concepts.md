---
title: Core Concepts
description: The concepts behind capture, sessions, blocks, selectors, and output modes.
---

## Capture

Capture is the act of turning terminal text into structured data that `sivtr` can browse, search, select, or copy.

`sivtr` supports three practical capture paths:

- stdin pipe: `command | sivtr`;
- subprocess wrapper: `sivtr run <command>`;
- shell session hook: `sivtr init <shell>` followed by normal terminal use.

Pipe and run mode are immediate. Shell integration builds a session log over time.

## Session log

A session log is a JSONL file containing command entries. Each entry stores:

- prompt;
- command;
- output;
- optional ANSI-preserved prompt;
- optional ANSI-preserved output.

The plain versions are used for stable copying, searching, and parsing. ANSI versions are kept when available so `--ansi` can preserve colors.

## Command block

A command block is the input and output for one command:

```text
PS C:\repo> cargo test
running 42 tests
test result: ok
```

`sivtr copy` can operate on the whole block, only the input, only the output, or the bare command.

## Selector

A selector chooses recent command blocks or Codex blocks.

| Selector | Meaning |
| --- | --- |
| `1` | Latest matching block |
| `2` | Second latest matching block |
| `2..4` | Range of recent blocks |

Selectors are intentionally relative to recency, because the most common task is to reuse what just happened.

## Open mode

Captured output can open in either:

- the built-in TUI browser;
- an external editor.

Configure this with:

```toml
[general]
open_mode = "tui"
```

or:

```toml
[general]
open_mode = "editor"
```
