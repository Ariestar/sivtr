---
title: Hotkey
description: Start, stop, and configure the Windows Codex picker hotkey.
---

The hotkey daemon is currently Windows-only. It registers one global shortcut and opens a new terminal window that runs the Codex picker for the working directory where the daemon was started.

## Start

```bash
sivtr hotkey start
```

Default chord:

```text
alt+y
```

Override it when starting:

```bash
sivtr hotkey start --chord ctrl+shift+y
```

Or configure it:

```toml
[hotkey]
chord = "alt+y"
```

## Check status

```bash
sivtr hotkey status
```

The status output includes:

- daemon pid;
- chord;
- working directory;
- executable path when available.

## Stop

```bash
sivtr hotkey stop
```

If the stored pid is stale, `sivtr` clears the state file.

## Behavior

When the chord is pressed, the daemon launches:

```bash
sivtr hotkey-pick-codex --cwd <daemon-working-directory>
```

That internal command opens the same selection workflow as:

```bash
sivtr copy codex --pick
```
