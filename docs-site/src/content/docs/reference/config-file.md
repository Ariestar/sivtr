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

If a legacy `sift/config.toml` exists, `sivtr` reads it for compatibility.

## Full example

```toml
[general]
open_mode = "tui"
preserve_colors = true

[editor]
command = "nvim"

[history]
auto_save = true
max_entries = 0

[copy]
prompts = ["PS C:\\repo> ", "dev>"]

[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]

[hotkey]
chord = "alt+y"
```

## general

```toml
[general]
open_mode = "tui"
preserve_colors = true
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `open_mode` | `"tui"` or `"editor"` | `"tui"` | Where captured output opens |
| `preserve_colors` | boolean | `true` | Preserve original ANSI colors in TUI display |

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

## copy

```toml
[copy]
prompts = []
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `prompts` | string array | `[]` | Prompt profiles or literal prefixes used when detecting command lines |

`prompt_presets` is a legacy field and is not serialized by the current config writer.

## codex

```toml
[codex]
session_dirs = []
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `session_dirs` | string array | `[]` | Extra exported Codex `sessions` directories to browse with `copy codex --pick` |

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | Chord used by `sivtr hotkey start` |
