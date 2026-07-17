---
title: Session Model
description: How shell command logs and agent-provider sessions become reusable memory blocks.
---

`sivtr` reuses local work by normalizing different sources into memory blocks that can be browsed, searched, copied, or shown by ref.

```text
source -> session -> block/dialogue -> selector/ref -> copy/search/show/diff
```

## Shell session entries

Shell integration records command blocks as structured JSONL entries. This gives `sivtr` a reliable source for copy, diff, import, and command-block navigation.

Each entry is a `SessionEntry`:

```json
{
  "prompt": "PS C:\\repo> ",
  "command": "cargo test",
  "output": "test result: ok",
  "prompt_ansi": "...",
  "output_ansi": "..."
}
```

`prompt_ansi` and `output_ansi` are omitted when identical to plain text or unavailable.

## Shell normalization

At construction and load boundaries, entries are normalized:

- CRLF is converted to LF;
- trailing newlines are trimmed;
- ANSI is stripped from plain prompt and output;
- ANSI content is preserved separately when different from plain text.

This allows stable plain-text copy, search, and parsing while keeping ANSI output available for `--ansi`.

## Rendering command input

The input portion is rendered from prompt plus command.

If the prompt ends with a newline, the command is placed on the next line. Otherwise, the command is appended to the last prompt line.

Example:

```text
PS C:\repo> cargo test
```

Multiline prompt:

```text
repo on main
> cargo test
```

## Agent session blocks

Provider parsers convert local transcript formats into a shared `AgentSession` shape:

```text
AgentSession
|- id
|- cwd
|- title
`- blocks
   |- User
   |- Assistant
   |- ToolCall
   `- ToolOutput
```

This lets copy, picker, search, and show logic work across registered providers (Codex, Claude Code, Cursor, Hermes, OpenCode, OpenClaw, Grok, Pi, …) without hard-coding one transcript format into the UI. The result is a provider-neutral memory layer rather than a one-off transcript reader.

## Current workspace resolution

Workspace commands resolve sessions relative to a working directory:

- provider sessions are filtered by recorded `cwd` when available;
- the current shell session is added when a session log exists;
- `--cwd` can override the directory for `search` and `show`.

Current-session environment hints such as `CODEX_THREAD_ID`, `CLAUDE_TRANSCRIPT_PATH`, and `CLAUDE_SESSION_ID` can help provider copy commands prefer the active local transcript.

## Why selectors are recency-based

The most common memory target is what just happened. Recency selectors make this cheap:

```bash
sivtr copy out      # latest output
sivtr copy out 2    # previous output
sivtr copy 2..4     # several recent blocks
sivtr copy claude 2 # previous agent turn
```

This avoids requiring users to remember absolute ids for transient terminal or agent work.

## Refs are for exact retrieval

Search results use refs when a stable follow-up memory target is needed:

```text
source/session[/dialogue[/line]]
```

Examples:

```bash
sivtr show claude/<session>/3
```

Selectors are for recent items. Refs are for exact items returned by search or picker workflows.

## Invalid shell logs

If a shell session log cannot be parsed as structured entries, `sivtr` resets the invalid log before appending new entries. This protects normal workflows from a corrupted or legacy file.
