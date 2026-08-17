---
title: Config File
description: TOML configuration reference.
---

## Location

`sivtr` uses the platform config directory:

| Platform | Current path |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

## Full example

```toml
[editor]
command = "nvim"

[history]
auto_save = true
max_entries = 0

[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]

[hotkey]
chord = "alt+y"
```

## editor

```toml
[editor]
command = "nvim"
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `command` | string | `""` | Editor command. Empty means auto-detect. |

Examples:

```toml
command = "hx"
command = "nvim"
command = "vim"
command = "code --wait"
```

## history

```toml
[history]
auto_save = true
max_entries = 0
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `auto_save` | boolean | `true` | Save captured output to history |
| `max_entries` | integer | `0` | Maximum entries to retain. `0` means unlimited. |

## codex

```toml
[codex]
session_dirs = []
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `session_dirs` | string array | `[]` | Extra exported Codex `sessions` directories to browse with `copy codex --pick` |

On macOS, a typical shared path is `/Users/Shared/sivtr/root-codex/sessions`.

Only Codex mirrors are currently configured here. Other registered providers (Claude, Cursor, OpenCode, OpenClaw, Hermes, Grok, Pi, …) use their own local locations and environment signals.

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | Chord used by `sivtr hotkey start` |
