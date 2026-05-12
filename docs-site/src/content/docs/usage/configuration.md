---
title: Configuration
description: Create, inspect, and edit sivtr configuration.
---

`sivtr` uses a TOML config file. The default path follows the platform config directory.

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

## Open in an editor by default

```toml
[general]
open_mode = "editor"

[editor]
command = "nvim"
```

When `open_mode` is `editor`, pipe mode, run mode, and session import open captured text in the configured external editor instead of the built-in TUI.

## Prompt detection

If your prompt is unusual, add literal prompt prefixes:

```toml
[copy]
prompts = ["dev>", "repo $", "PS C:\\repo>"]
```

This helps command-block parsing identify the command input lines in session logs.

## Shared Codex session trees

Add shared exported session trees when another account publishes a read-only copy:

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

Create that shared tree from the source account with:

```bash
sivtr codex export --dest /srv/sivtr/root-codex
```

On macOS, a shared path under `/Users/Shared` works well:

```toml
[codex]
session_dirs = ["/Users/Shared/sivtr/root-codex/sessions"]
```
