---
title: Agent Sessions
description: Turn Codex, Claude Code, OpenCode, and Pi sessions into reusable agent memory.
---

`sivtr` treats agent transcripts as local workspace memory sources. You can copy the latest useful turn, browse older sessions in a picker, search across providers, and show exact refs without opening raw transcript files. Prior agent work becomes memory for both humans and later agents.

Skills are how later agents learn to use this memory. A `sivtr` memory skill can instruct an agent to search local terminal and agent history, expand only the smallest relevant result, and verify current code before trusting prior discussion. See [Skills and Reusable Procedures](/usage/skills/).

## Supported providers

| Provider | Copy command | Default discovery |
| --- | --- | --- |
| Codex | `sivtr copy codex ...` | Codex rollout JSONL files under `~/.codex/sessions` |
| Claude Code | `sivtr copy claude ...` | Claude transcript/session environment and local transcripts |
| OpenCode | `sivtr copy opencode ...` | OpenCode local database |
| Pi | `sivtr copy pi ...` | Pi session JSONL files under the Pi agent directory |

Use `agent` in search commands when you want all supported providers:

```bash
sivtr search agent --match "panic" --format timeline
```

## Copy the latest useful text

```bash
sivtr copy codex out
sivtr copy claude out
sivtr copy opencode out
sivtr copy pi out
```

`out` copies the latest assistant reply for the selected provider.

## Copy input, output, tool output, or whole session

Each provider supports the same mode shape:

```bash
sivtr copy claude
sivtr copy claude in
sivtr copy claude out
sivtr copy claude tool
sivtr copy claude all
```

| Mode | Copies |
| --- | --- |
| omitted | Last completed user plus assistant turn |
| `in` | Last user message |
| `out` | Last assistant reply |
| `tool` | Last tool output |
| `all` | Whole parsed session |

Replace `claude` with `codex`, `opencode`, or `pi`.

## Select older items

Selectors are newest-first and work like command-block selectors:

```bash
sivtr copy claude 2
sivtr copy claude 2..4
sivtr copy codex out 3
```

Use `--session` to choose a session by picker index, session id, or id prefix:

```bash
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy claude out --session 3 --print
```

## Filter copied text

```bash
sivtr copy claude tool --regex error
sivtr copy codex all --lines 1:40
sivtr copy pi out --print
```

Filters run after selected text is assembled. See [Selectors and Filters](/reference/selectors-and-filters/).

## Pick interactively

Open a provider-specific picker:

```bash
sivtr copy codex --pick
sivtr copy claude --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

Inside the workspace picker:

| Key | Action |
| --- | --- |
| `0` / `1` / `2` / `3` | Focus source, sessions, dialogues, or content |
| `h` / `l` | Move between panes |
| `j` / `k` | Move within the current pane |
| `Space` | Toggle the current source, session, or dialogue |
| `/` | Search all loaded sessions |
| `n` / `N` | Next or previous search match |
| `i` | Copy current input/question |
| `o` | Copy current output/answer |
| `y` | Copy current input plus output |
| `c` | Copy bare command when available |
| `:` | Start a temporary line filter for the next copy |
| `v` in Content | Start visual text selection |
| `t` | Open current content in Vim-style full view |
| `?` | Open help |

Context-aware launchers such as the Windows hotkey and VS Code extension can open a current-workspace picker directly.

## Search agent memory and terminal context

Search current-workspace agent sessions and terminal context with target-first search:

```bash
sivtr search agent --match "panic" --format timeline
sivtr search agent --match "workspace picker" --in title --format compact
sivtr search codex --match "sivtr" --in session --format md
sivtr search terminal --match "build error" --format json --limit 20
```

Use JSON refs with `sivtr show`. These exact refs are the unit you can hand back to yourself, another terminal workflow, or an agent prompt:

```bash
sivtr show claude/<session>/<dialogue>
sivtr show pi/<session>/<dialogue>/<line>
```

## Codex session mirrors

Codex supports exporting local rollout JSONL files into a shared read-only tree:

```bash
sivtr codex export --dest /srv/sivtr/root-codex --watch
```

Then add the exported `sessions` directory to another account's config:

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

Shared/mirrored session trees only participate in explicit picker browsing. Implicit current-session lookup stays local so another account's exported history does not override your active workflow.

On macOS, `/Users/Shared/sivtr/root-codex` is a good shared location between local accounts:

```bash
sivtr codex export --dest /Users/Shared/sivtr/root-codex --watch
```

```toml
[codex]
session_dirs = ["/Users/Shared/sivtr/root-codex/sessions"]
```
