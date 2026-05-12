# sivtr VS Code extension

Launch the sivtr Codex picker from VS Code.

## Usage

Install `sivtr` first:

```bash
cargo install sivtr
```

If `sivtr` is missing, the extension will offer to open a terminal and run that
install command.

Run `Sivtr: Pick Codex Turn` from the command palette.

Default keybinding:

- Linux / Windows: `Alt+Y`
- macOS: `Cmd+Alt+Y`

The extension opens a VS Code terminal in the current workspace and runs the
context-aware Codex picker:

```bash
sivtr hotkey-pick-codex --cwd .
```

The extension resolves the workspace path before launch, so `--cwd .`,
`--cwd=./subdir`, and `${workspaceFolder}` all expand to the current workspace.
It also quotes arguments for the active terminal shell, which keeps macOS
setups working when the workspace path contains spaces or when VS Code uses
`zsh`, `bash`, or `fish`.

If the terminal was opened from a live `codex resume` session, `sivtr` prefers
that exact session id first. Otherwise it falls back to the newest non-empty
session whose `cwd` matches the workspace.

On Linux, this VS Code keybinding is the recommended default shortcut. `sivtr`
does not currently provide a desktop-wide global hotkey on Linux outside VS
Code because global shortcut registration and terminal launching are not
portable across Wayland, X11, and terminal-only environments.

On macOS, the same VS Code shortcut is also the recommended default shortcut.
If you want a Terminal-based launcher outside VS Code, run
`sivtr init macos-shortcut` on the Mac host.

Quick one-line fallback outside VS Code:

```bash
sivtr init macos-shortcut && ~/.local/bin/sivtr-pick-codex
```

## Settings

| Setting | Default | Purpose |
| --- | --- | --- |
| `sivtr.command` | `sivtr` | Command used to launch sivtr |
| `sivtr.args` | `["hotkey-pick-codex", "--cwd", "."]` | Arguments passed to sivtr |
| `sivtr.reuseTerminal` | `true` | Reuse the existing sivtr terminal |
| `sivtr.closeTerminalOnSuccess` | `true` | Close the sivtr terminal when the picker exits successfully |
| `sivtr.terminalName` | `sivtr` | Terminal name |

On macOS, keep the default `Cmd+Alt+Y` unless you already use that chord for
another editor command. If you override `sivtr.args`, prefer `--cwd .` or
`${workspaceFolder}` instead of hard-coded quoted paths.

## Development

```bash
pnpm install
pnpm run compile
pnpm run test
pnpm run package
```

Open this folder in VS Code and press `F5` to launch an Extension Development Host.

## License

Apache License 2.0. See the repository [LICENSE](../../LICENSE).
