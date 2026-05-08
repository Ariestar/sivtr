---
title: Hotkey
description: Start, stop, and configure the Windows AI session picker hotkey.
---

The hotkey daemon is currently Windows-only. It registers one global shortcut and opens a new terminal window that runs the AI session picker for the working directory where the daemon was started.

Linux does not currently ship a default desktop-wide `sivtr` hotkey. Wayland
does not expose a universal cross-desktop global hotkey API for ordinary CLI
apps, and launching the picker also requires an interactive terminal that is
not portable across GNOME, KDE, Sway, tmux, and headless SSH sessions.

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

## Linux setups

Use one of these platform-specific entry points instead of a built-in global
daemon:

- VS Code: use the extension command bound to `Alt+Y` by default.
- tmux: install the helper block and reload tmux:

```bash
sivtr init tmux
tmux source-file ~/.tmux.conf
```

This binds `prefix + y` to the current pane path.

- Desktop shortcut or terminal launcher: generate a project-specific launcher:

```bash
sivtr init linux-shortcut
```

This writes `~/.local/bin/sivtr-pick-codex` plus a desktop entry at
`~/.local/share/applications/sivtr-pick-codex.desktop`. Bind your desktop
shortcut to the script, or run it directly from a terminal.

- macOS: generate a Terminal launcher plus a LaunchAgent wrapper:

```bash
sivtr init macos-shortcut
```

This writes `~/.local/bin/sivtr-pick-codex` and
`~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`. Run the script directly,
or load the LaunchAgent with:

```bash
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.sivtr.pick-codex.plist
```
