# sivtr VS Code extension

Launch the sivtr Codex picker from VS Code.

## Usage

Install `sivtr` first:

```bash
cargo install sivtr
```

If `sivtr` is missing, the extension will offer to open a terminal and run that
install command.

Run `Sivtr: Pick Codex Turn` from the command palette, or press `Alt+Y`.

The extension opens a VS Code terminal in the current workspace and runs the
context-aware Codex picker:

```bash
sivtr hotkey-pick-codex --cwd .
```

## Settings

| Setting | Default | Purpose |
| --- | --- | --- |
| `sivtr.command` | `sivtr` | Command used to launch sivtr |
| `sivtr.args` | `["hotkey-pick-codex", "--cwd", "."]` | Arguments passed to sivtr |
| `sivtr.reuseTerminal` | `true` | Reuse the existing sivtr terminal |
| `sivtr.closeTerminalOnSuccess` | `true` | Close the sivtr terminal when the picker exits successfully |
| `sivtr.terminalName` | `sivtr` | Terminal name |

To use `Ctrl+Y`, override the keybinding in VS Code Keyboard Shortcuts.

## Development

```bash
pnpm install
pnpm run compile
pnpm run package
```

Open this folder in VS Code and press `F5` to launch an Extension Development Host.

## License

Apache License 2.0. See the repository [LICENSE](../../LICENSE).
