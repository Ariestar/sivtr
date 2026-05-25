---
title: CLI Reference
description: Command syntax, subcommands, options, providers, selectors, and examples.
---

This page documents the public CLI surface. The source of truth is `src/cli.rs`; run `sivtr --help` and `sivtr <command> --help` for installed-version help.

## Top-level

```bash
sivtr [COMMAND]
```

If no command is provided, `sivtr` reads from stdin, matching pipe mode.

## run

```bash
sivtr run <COMMAND> [ARGS...]
```

Runs a command, captures combined stdout/stderr, reports the exit status, saves history when enabled, and opens the captured output.

```bash
sivtr run cargo test
sivtr run git status --short
```

## pipe

```bash
sivtr pipe
```

Reads stdin and opens it. Piping directly to `sivtr` is equivalent:

```bash
cargo build 2>&1 | sivtr
```

## import

```bash
sivtr import
```

Opens the current structured shell session log. Requires shell integration.

## init

```bash
sivtr init <TARGET>
```

Supported targets:

| Target | Purpose |
| --- | --- |
| `powershell` | Install Windows PowerShell hook |
| `pwsh` | Alias for PowerShell integration |
| `bash` | Install Bash hook |
| `zsh` | Install Zsh hook |
| `nushell` / `nu` | Install Nushell hook |
| `tmux` | Install tmux picker binding |
| `linux-shortcut` | Generate Linux desktop/terminal picker launcher |
| `macos-shortcut` | Generate macOS Terminal/LaunchAgent picker launcher |

## copy

```bash
sivtr copy [MODE] [SELECTOR] [OPTIONS]
```

Command-block modes:

| Mode | Meaning |
| --- | --- |
| no mode | Copy input plus output |
| `in` | Copy input |
| `out` | Copy output |
| `cmd` | Copy bare command |

Aliases:

| Alias | Expands to |
| --- | --- |
| `sivtr c` | `sivtr copy` |
| `sivtr ci` | `sivtr copy in` |
| `sivtr co` | `sivtr copy out` |
| `sivtr cc` | `sivtr copy cmd` |

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

## copy agent provider sessions

```bash
sivtr copy <PROVIDER> [MODE] [SELECTOR] [OPTIONS]
```

Providers:

| Provider | Command |
| --- | --- |
| Codex | `sivtr copy codex` |
| Claude Code | `sivtr copy claude` |
| OpenCode | `sivtr copy opencode` |
| Pi | `sivtr copy pi` |

Modes:

| Mode | Meaning |
| --- | --- |
| no mode | Last completed user + assistant turn |
| `in` | Last user message |
| `out` | Last assistant reply |
| `tool` | Last tool output |
| `all` | Whole parsed session |

Agent copy options include all common copy options plus:

| Option | Meaning |
| --- | --- |
| `--session <N|ID>` | Select the Nth newest selectable session, or match an id/id prefix |

Examples:

```bash
sivtr copy claude
sivtr copy claude out --print
sivtr copy claude --session 2
sivtr copy codex 2..4
sivtr copy codex out --pick
sivtr copy opencode all --lines 1:20
sivtr copy pi tool --regex error
```

## diff

```bash
sivtr diff <LEFT> <RIGHT> [OPTIONS]
```

Compares two recent command blocks from the current shell session. Each selector must resolve to exactly one block.

Content options:

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

## search

```bash
sivtr search <TARGET> [OPTIONS]
```

Searches captured terminal records and supported AI workspace sessions. The target chooses where to search; filters choose which records match.

Targets:

| Target | Meaning |
| --- | --- |
| `terminal[/<session>[/<record>[/<line>]]]` | Terminal command records |
| `agent[/<session>[/<turn>[/<line>]]]` | All supported AI/agent records |
| `codex[/<session>[/<turn>[/<line>]]]` | Codex records |
| `claude[/<session>[/<turn>[/<line>]]]` | Claude Code records |
| `opencode[/<session>[/<turn>[/<line>]]]` | OpenCode records |
| `pi[/<session>[/<turn>[/<line>]]]` | Pi records |

Use `*` for wildcard path segments, for example `terminal/*/3` or `pi/*/*`.

Options:

| Option | Meaning |
| --- | --- |
| `--match <REGEX>` | Case-insensitive content filter |
| `--exclude <REGEX>` | Case-insensitive exclusion filter applied after matches are found |
| `--in <FIELD>` | `content`, `title`, `session`, `input`, `output`, `command`, or `all`; default is `content` |
| `--status <STATUS>` | `success`, `failure`, or `unknown` |
| `--exit-code <CODE>` | Exact terminal process exit code |
| `--min-duration <DURATION>` | Minimum command duration, e.g. `500ms`, `2s`, `1m` |
| `--max-duration <DURATION>` | Maximum command duration |
| `--sort <SORT>` | `newest`, `oldest`, `duration`, `duration-asc`, `exit-code`, or `exit-code-asc` |
| `--cwd <PATH>` | Workspace directory used to resolve records |
| `--since <TIME>` | Only include records at or after this time |
| `--until <TIME>` | Only include records at or before this time |
| `--last <DURATION>` | Recent time window, e.g. `30m`, `2h`, `7d` |
| `--latest <N>` | Return the latest N matching records before final sort |
| `-l, --limit <N>` | Maximum result groups to print |
| `--exclude-current`, `--other` | Exclude the current agent session from agent searches |
| `--json` | Alias for `--format json` |
| `--format <FORMAT>` | `timeline`, `compact`, `md`, or `json`; default is `json` |

Time filters accept RFC3339 timestamps, Unix seconds/milliseconds, relative durations like `30m`, `2h`, `7d`, and aliases such as `today`, `yesterday`, `tomorrow`, `this morning`, `this afternoon`, `this evening`, `tonight`, and `now`.

Examples:

```bash
sivtr search terminal --status failure --latest 1 --json
sivtr search terminal --match "panic|failed" --exclude "example|sample" --since today --format timeline
sivtr search agent --match "TODO|failed|next step" --since yesterday --format md
sivtr search pi --since today --sort oldest --format timeline
sivtr search pi/019e5941 --match "cargo test" --format compact
sivtr search terminal/session_13104/3/12 --format json
```

## show

```bash
sivtr show <REF> [OPTIONS]
```

Prints a workspace ref from an agent provider or the current terminal session.

Ref syntax:

```text
source/session[/dialogue[/line]]
```

Options:

| Option | Meaning |
| --- | --- |
| `--cwd <PATH>` | Workspace directory used to resolve sessions |
| `--json` | Print machine-readable JSON |

Examples:

```bash
sivtr show claude/<session-id>
sivtr show claude/<session-id>/3
sivtr show claude/<session-id>/3/7 --json
sivtr show terminal/current/2
```

## version

```bash
sivtr version [--verbose]
```

Prints the Sivtr version. Use `--verbose` to diagnose which binary is running and whether it differs from the local debug build in the current repository.

```bash
sivtr version
sivtr version --verbose
```

Verbose output includes:

- package version;
- binary path;
- current working directory;
- debug/release profile;
- git commit and build time when available;
- detected repo root;
- local `target/debug/sivtr` binary status;
- a warning when a different global binary is being used inside the repo.

## history

```bash
sivtr history [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `list [-l, --limit <N>]` | List recent entries |
| `search <KEYWORD> [-l, --limit <N>]` | Search saved capture history |
| `show <ID>` | Show a specific history entry |

If no history subcommand is provided, `list` is used.

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
| `start [--chord <CHORD>] [--provider <PROVIDER>]` | Start Windows global hotkey daemon |
| `status` | Show daemon status |
| `stop` | Stop daemon |

If no hotkey subcommand is provided, `status` is used.

Examples:

```bash
sivtr hotkey start
sivtr hotkey start --chord alt+y
sivtr hotkey start --provider claude
sivtr hotkey status
sivtr hotkey stop
```

## codex export

```bash
sivtr codex export --dest <PATH> [OPTIONS]
```

Exports local Codex rollout JSONL files into a target directory containing a `sessions/` tree.

Options:

| Option | Meaning |
| --- | --- |
| `--dest <PATH>` | Destination directory that will receive the `sessions/` tree |
| `--limit <N>` | Keep only newest N session files; `0` means export all |
| `--watch` | Continue mirroring local sessions |
| `--interval <SECONDS>` | Seconds between sync passes when watching; default is `1` |
| `--interval-ms <MILLISECONDS>` | Milliseconds between sync passes; overrides `--interval` |

Examples:

```bash
sivtr codex export --dest /srv/sivtr/root-codex
sivtr codex export --dest /srv/sivtr/root-codex --watch
sivtr codex export --dest /srv/sivtr/root-codex --limit 100
```

## clear

```bash
sivtr clear [--all]
```

Clears current shell session logs. `--all` clears all recorded session logs and state files managed by `sivtr`.

## Shared syntax

See [Selectors and Filters](/reference/selectors-and-filters/) for recency selectors, `--session`, providers, `--regex`, `--lines`, `--ansi`, `--print`, and workspace refs.
