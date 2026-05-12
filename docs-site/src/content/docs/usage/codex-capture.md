---
title: Codex Capture
description: Copy useful blocks from the current Codex session.
---

`sivtr copy codex` reads Codex rollout JSONL files from `~/.codex/sessions`. If the current shell exports `CODEX_THREAD_ID`, `sivtr` prefers that exact local session first. Otherwise it chooses the newest local session whose `cwd` matches the current working directory.

If another account publishes a read-only mirror with `sivtr codex export --dest ...`, add that mirrored `sessions` directory to `[codex].session_dirs` so explicit `--pick` browsing can read it without elevated privileges.

Use `--session N` to select the Nth newest recorded session, or `--session ID` to match a session id / id prefix explicitly.

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
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy codex 2
sivtr copy codex 2..4
sivtr copy codex out --session 3
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
sivtr copy codex --session 2 --pick
sivtr copy codex --pick
sivtr copy codex out --pick
sivtr copy codex --pick  # includes mirrored session trees from [codex].session_dirs
```

The plain CLI picker starts with the session list, then lets you choose one or more units from that session. Press `t` to open the Vim-style view. In Codex views, `T` toggles tool content when an alternate full view is available.

Context-aware launchers such as the Windows hotkey and VS Code extension first open the newest non-empty session for the current workspace. If that session is missing or empty, they fall back to the session list.

Shared/mirrored session trees only participate in explicit `--pick` browsing. Implicit current-session lookup stays local so another account's exported history does not override the current user's active Codex workflow.

## Mirror sessions for another account

Create a shared mirror from the source account:

```bash
sivtr codex export --dest /srv/sivtr/root-codex --watch
```

Then consume it from another account:

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

On macOS, `/Users/Shared/sivtr/root-codex` is a good shared location between
local accounts:

```bash
sivtr codex export --dest /Users/Shared/sivtr/root-codex --watch
```

```toml
[codex]
session_dirs = ["/Users/Shared/sivtr/root-codex/sessions"]
```

Quick one-line checks:

- export side: `rm -rf /Users/Shared/sivtr/root-codex-smoke && sivtr codex export --dest /Users/Shared/sivtr/root-codex-smoke && find /Users/Shared/sivtr/root-codex-smoke -maxdepth 2 -type f | sed -n '1,5p'`
- read side after configuring `[codex].session_dirs`: `sivtr copy codex --pick`

## Windows hotkey

On Windows, the hotkey daemon opens the AI session picker for the project directory where it was started:

```bash
sivtr hotkey start
```

The default chord is `alt+y`. Configure it in `[hotkey]` or pass `--chord` when starting.
