---
title: History
description: List, search, and show saved output history.
---

`sivtr` stores captured output in a local SQLite history database with FTS5 search. The history commands are read-oriented: list recent entries, search by keyword, and show a full entry.

## List recent entries

```bash
sivtr history
sivtr history list
sivtr history list --limit 50
```

Output includes the entry id, timestamp, command, and a preview of the content.

## Search

```bash
sivtr history search "panic"
sivtr history search "failed assertion" --limit 10
```

Search uses the history full-text index. Use the resulting id with `history show`.

## Show an entry

```bash
sivtr history show 42
```

The detail view prints metadata followed by the stored content:

- id;
- timestamp;
- command;
- source;
- host;
- content.

## Retention

History retention is controlled by config:

```toml
[history]
auto_save = true
max_entries = 0
```

`max_entries = 0` means unlimited.
