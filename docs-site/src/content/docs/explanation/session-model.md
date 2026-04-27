---
title: Session Model
description: How shell integration records and reuses command blocks.
---

Shell integration records command blocks as structured JSONL entries. This gives `sivtr` a reliable source for copy, diff, import, and command-block navigation.

## Entry shape

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

`prompt_ansi` and `output_ansi` are omitted when they are identical to plain text or unavailable.

## Normalization

At construction and load boundaries, entries are normalized:

- CRLF is converted to LF;
- trailing newlines are trimmed;
- ANSI is stripped from plain prompt and output;
- ANSI content is preserved separately when different from plain text.

This allows stable plain-text operations while keeping ANSI output available for `--ansi`.

## Rendering input

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

## Why selectors are recency-based

The most common reuse target is what just happened. Recency selectors make this cheap:

```bash
sivtr copy out      # latest output
sivtr copy out 2    # previous output
sivtr copy 2..4     # several recent blocks
```

This avoids requiring users to remember absolute ids for transient terminal work.

## Invalid logs

If a session log cannot be parsed as structured entries, `sivtr` resets the invalid log before appending new entries. This protects normal workflows from a corrupted or legacy file.

## Legacy compatibility

The config path resolver checks the current `sivtr` config path first. If no current config exists but a legacy `sift/config.toml` exists, it reads the legacy file.
