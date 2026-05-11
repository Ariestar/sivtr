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

## Linux desktop shortcut (manual)

Linux does not currently expose one universal global-hotkey API for CLI apps
across GNOME/KDE/Wayland/X11. Use a launcher script and bind it in your
desktop settings.

1. Create a launcher script:

```bash
mkdir -p ~/.local/bin
cat > ~/.local/bin/sivtr-pick-codex <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
export PROJECT_CWD="$HOME"
# Optional cross-account mirror:
# export SIVTR_CODEX_SESSION_DIRS='/srv/sivtr/root-codex/sessions:/home/<user>/codex_transfer/sessions'
exec x-terminal-emulator -e bash -lc 'cd "$PROJECT_CWD"; exec sivtr copy codex --pick'
EOF
chmod +x ~/.local/bin/sivtr-pick-codex
```

2. Bind a shortcut key to `~/.local/bin/sivtr-pick-codex`.
   GNOME path: `Settings -> Keyboard -> Keyboard Shortcuts -> View and
   Customize Shortcuts -> Custom Shortcuts`.
   KDE path: `System Settings -> Shortcuts -> Custom Shortcuts`.

3. Press your key chord (for example `Ctrl+Alt+Q`) to open the picker.

## Other terminal shortcuts

If you prefer terminal-native shortcuts:

- tmux:

```tmux
bind-key y new-window -c "#{pane_current_path}" "sivtr copy codex --pick"
```

- WezTerm / Kitty / Alacritty / Ghostty: bind a key to run
  `~/.local/bin/sivtr-pick-codex`.
- One-off command in any terminal:

```bash
sivtr copy codex --pick
```
