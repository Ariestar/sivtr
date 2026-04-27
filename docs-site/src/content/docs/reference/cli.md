---
title: CLI Reference
description: Command syntax, subcommands, options, selectors, and examples.
---

This page documents the public CLI surface. Keep it aligned with `src/cli.rs`.

## Top-level

```bash
sivtr [COMMAND]
```

If no command is provided, `sivtr` reads from stdin, matching pipe mode.

## run

```bash
sivtr run <COMMAND> [ARGS...]
```

Runs a command, captures combined output, reports the exit status, and opens the captured output.

Examples:

```bash
sivtr run cargo test
sivtr run git status --short
```

## pipe

```bash
sivtr pipe
```

Reads stdin and opens it. In normal use, piping directly to `sivtr` is equivalent:

```bash
cargo build 2>&1 | sivtr
```

## import

```bash
sivtr import
```

Opens the current structured session log. Requires shell integration.

## init

```bash
sivtr init <SHELL>
```

Supported shell names:

- `powershell`
- `pwsh`
- `bash`
- `zsh`
- `nushell`
- `nu`

## copy

```bash
sivtr copy [MODE] [SELECTOR] [OPTIONS]
```

Modes:

| Mode | Meaning |
| --- | --- |
| no mode | Copy input plus output |
| `in` | Copy input |
| `out` | Copy output |
| `cmd` | Copy bare command |
| `codex` | Copy Codex session content |

Aliases:

| Alias | Expands to |
| --- | --- |
| `c` | `copy` |
| `ci` | `copy in` |
| `co` | `copy out` |
| `cc` | `copy cmd` |

Common options:

| Option | Meaning |
| --- | --- |
| `--ansi` | Copy ANSI-decorated text when available |
| `--pick` | Open the interactive picker |
| `--print` | Print copied text after copying |
| `--regex <PATTERN>` | Keep lines matching regex |
| `--lines <SPEC>` | Keep selected 1-based lines |

Input-capable modes also support:

| Option | Meaning |
| --- | --- |
| `--prompt <TEXT>` | Rewrite the copied input prompt |

Examples:

```bash
sivtr copy
sivtr copy 3 --print
sivtr copy --prompt ":"
sivtr copy in 2..4
sivtr copy out --pick --regex panic
sivtr copy cmd --pick
```

## copy codex

```bash
sivtr copy codex [MODE] [SELECTOR] [OPTIONS]
```

Modes:

| Mode | Meaning |
| --- | --- |
| no mode | Last completed user plus assistant turn |
| `out` | Last assistant reply |
| `in` | Last user message |
| `tool` | Last tool output |
| `all` | Whole parsed session |

Examples:

```bash
sivtr copy codex
sivtr copy codex 2
sivtr copy codex 2..4
sivtr copy codex out --print
sivtr copy codex out --pick
sivtr copy codex tool --regex error
sivtr copy codex all --lines 1:20
```

## diff

```bash
sivtr diff <LEFT> <RIGHT> [OPTIONS]
```

Compares two recent command blocks from the current session. Each selector must resolve to exactly one block.

Content modes:

| Option | Meaning |
| --- | --- |
| `--output` | Compare output text. This is the default. |
| `--block` | Compare input plus output |
| `--input` | Compare input with prompt |
| `--cmd` | Compare bare command text |

View option:

| Option | Meaning |
| --- | --- |
| `--side-by-side` | Show a two-column text view |

Examples:

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
sivtr diff 2 1 --side-by-side
```

## history

```bash
sivtr history [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `list [-l, --limit <N>]` | List recent entries |
| `search <KEYWORD> [-l, --limit <N>]` | Search history |
| `show <ID>` | Show a specific entry |

If no history subcommand is provided, `sivtr` lists the latest 20 entries.

## config

```bash
sivtr config [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `show` | Show config path and content |
| `init` | Create default config |
| `edit` | Open config in editor |

If no config subcommand is provided, `show` is used.

## hotkey

```bash
sivtr hotkey [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `start [--chord <CHORD>]` | Start Windows hotkey daemon |
| `status` | Show daemon status |
| `stop` | Stop daemon |

If no hotkey subcommand is provided, `status` is used.

## clear

```bash
sivtr clear [--all]
```

Clears session logs. `--all` clears all recorded session logs and state files.

## Selector syntax

| Selector | Meaning |
| --- | --- |
| omitted | `1` |
| `1` | Latest matching item |
| `2` | Second latest matching item |
| `2..4` | Recent range |

Selector semantics are shared by command-block copy, Codex copy, and diff where applicable.
