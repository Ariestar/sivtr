---
title: Configuration
description: Create, inspect, edit, and understand sivtr configuration.
---

`sivtr` uses a TOML config file in the platform config directory. Configuration controls open mode, editor handoff, history retention, prompt detection, Codex mirrors, and the Windows hotkey chord.

## Commands

```bash
sivtr config show
sivtr config init
sivtr config edit
```

| Command | Behavior |
| --- | --- |
| `sivtr config show` | Print config path and effective file content or defaults |
| `sivtr config init` | Create the default config if it does not exist |
| `sivtr config edit` | Create the config if needed and open it in the configured editor |

## Default config

```toml
[general]
open_mode = "tui"
preserve_colors = true

[editor]
command = ""

[history]
auto_save = true
max_entries = 0

[copy]
prompts = []

[codex]
session_dirs = []

[hotkey]
chord = "alt+y"
```

For a field-by-field reference, see [Config File](/reference/config-file/).

## Open captured output in an editor

```toml
[general]
open_mode = "editor"

[editor]
command = "nvim"
```

When `open_mode` is `editor`, pipe mode, run mode, and session import open captured text in the configured external editor instead of the built-in TUI.

## Preserve colors

```toml
[general]
preserve_colors = true
```

When enabled, the TUI can display ANSI colors where captured ANSI content is available. Plain-text copy and search remain stable.

## Prompt detection

If your prompt is unusual, add literal prompt prefixes:

```toml
[copy]
prompts = ["dev>", "repo $", "PS C:\\repo>"]
```

This helps command-block parsing identify command input lines in session logs.

## History retention

```toml
[history]
auto_save = true
max_entries = 0
```

`max_entries = 0` means unlimited. Set `auto_save = false` when you do not want pipe and run captures written to history automatically.

## Shared Codex session trees

Add shared exported Codex session trees when another account publishes a read-only copy:

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

Create that shared tree from the source account with:

```bash
sivtr codex export --dest /srv/sivtr/root-codex
sivtr codex export --dest /srv/sivtr/root-codex --watch
```

Only Codex currently has first-class shared mirror configuration. Other registered providers are read from their local provider-specific locations. See [Data Locations](/reference/data-locations/).

## Hotkey chord

```toml
[hotkey]
chord = "alt+y"
```

This chord is used by `sivtr hotkey start` unless overridden with `--chord`.

Provider selection is a runtime CLI option, not a config key:

```bash
sivtr hotkey start --provider all
sivtr hotkey start --provider claude
```
