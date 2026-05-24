---
title: sivtr
description: A shared memory workspace for humans and agents.
---

`sivtr` is a local-first shared memory workspace for humans and agents. It turns the work around a project—terminal commands, command output, AI-agent conversations, tool results, and copied context—into searchable, selectable, referenceable memory that both you and your agents can reuse.

Use it beside the terminals and agents you already have. It gives their local work one shared memory workspace.

## What sivtr is not

`sivtr` is not:

- a terminal emulator;
- a tmux replacement;
- a hosted transcript service;
- another agent runtime.

## What sivtr helps with

- **Capture working memory** from pipes, subprocesses, shell integration, and local agent transcripts.
- **Browse and select text** in a keyboard-first Vim-style TUI.
- **Copy recent command blocks** as input, output, bare command, or full block.
- **Reuse agent dialogue** from Codex, Claude Code, OpenCode, or Pi as project memory.
- **Teach agents to use the same memory** through skills and reusable procedures, so "fix the terminal error" can start from local evidence.
- **Search and show workspace refs** across terminal context and agent-session records.
- **Hand exact context to humans or agents** with stable refs, selectors, filters, and copy modes.
- **Launch memory pickers** from the terminal, tmux, VS Code, Windows hotkeys, or generated desktop shortcuts.

## First useful commands

```bash
# Browse command output as reusable workspace memory.
cargo test 2>&1 | sivtr

# Let sivtr run the command and capture combined output.
sivtr run cargo test

# Copy the latest recorded command output.
sivtr copy out

# Copy the latest assistant answer from an agent provider.
sivtr copy claude out
sivtr copy codex out
sivtr copy opencode out
sivtr copy pi out

# Search current workspace memory.
sivtr search agent --match "panic" --format timeline
```

## Common workflows

| Goal | Start here |
| --- | --- |
| Install the CLI | [Installation](/start/installation/) |
| Learn the daily path | [Quickstart](/start/quickstart/) |
| Understand the model | [Mental Model](/start/core-concepts/) |
| Capture output | [Capture Terminal Output](/usage/capture-output/) |
| Copy recent commands | [Copy Command Blocks](/usage/copy-command-blocks/) |
| Reuse agent memory | [Work with AI Sessions](/usage/ai-sessions/) |
| Teach agents the memory workflow | [Skills and Reusable Procedures](/usage/skills/) |
| See practical community workflows | [Playbooks](/playbooks/) |
| Search and dereference memory | [Search and Show Results](/usage/search-and-show/) |
| Open pickers quickly | [Launch Pickers and Hotkeys](/usage/launchers-and-hotkeys/) |
| Check exact syntax | [CLI Reference](/reference/cli/) |

## Mental model

`sivtr` has two layers:

| Layer | What it describes |
| --- | --- |
| Memory layer | Terminal records, agent conversations, sessions, dialogues, command blocks, and refs. |
| Use layer | TUI browsing, search, copy, show, diff, skills, and playbooks. |

Terminal sources produce command blocks. Agent providers produce dialogue blocks. Selectors like `1` and `2..4` pick recent memory for copy commands. Refs like `claude/<session>/3/2` identify exact search results for `sivtr show`.

## Local by default

`sivtr` reads local shell logs, local history, and local agent transcripts. Shared Codex trees are opt-in through explicit export and configuration. See [Data Locations](/reference/data-locations/) for where records live.
