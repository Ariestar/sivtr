---
title: Hotkey
description: Start, stop, and configure the Windows AI session picker hotkey.
---

The hotkey daemon is currently Windows-only. It registers one global shortcut and opens a new terminal window that runs the AI session picker for the working directory where the daemon was started.

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
sivtr hotkey-pick-agent --cwd <daemon-working-directory> --provider all
```

That internal command first opens the newest non-empty Codex session for the
daemon working directory. If that session is missing or empty, it falls back to
the session picker.

Plain `sivtr copy codex --pick` is different: it always starts with the session
picker.
