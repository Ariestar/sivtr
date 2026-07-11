---
title: CLI Reference
description: Command syntax, subcommands, options, providers, selectors, and examples.
---

This page documents the public CLI surface. The source of truth is `src/cli/`; run `sivtr --help` and `sivtr <command> --help` for installed-version help.

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
| Hermes | `sivtr copy hermes` |
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
sivtr copy hermes out --print
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
| `hermes[/<session>[/<turn>[/<line>]]]` | Hermes records |
| `opencode[/<session>[/<turn>[/<line>]]]` | OpenCode records |
| `pi[/<session>[/<turn>[/<line>]]]` | Pi records |
| `<origin>:<target>` | Remote or other-workspace origin, for example `desk:terminal` or `docs:codex/4` |

Use `*` for wildcard path segments, for example `terminal/*/3` or `pi/*/*`. Origins come from `sivtr remote add <alias> ...` mounts or local workspace names listed by `sivtr wb list`.

Options:

| Option | Meaning |
| --- | --- |
| `--match <REGEX>`, `-m <REGEX>` | Case-insensitive content filter |
| `--exclude <REGEX>`, `-v <REGEX>` | Case-insensitive exclusion filter applied after matches are found |
| `--in <FIELD>`, `-i <FIELD>` | `content`, `title`, `session`, `input`, `output`, `command`, or `all`; default is `content` |
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
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs`; prints refs, one per line |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset`; terminal stdout defaults to `full`, piped stdout defaults to `workset` |

When stdout is piped and no explicit format is selected, WorkSet commands emit WorkSet JSON for the next command. Use `--refs` or `-f timeline` only at the final display step.

Time filters accept RFC3339 timestamps, Unix seconds/milliseconds, relative durations like `30m`, `2h`, `7d`, and aliases such as `today`, `yesterday`, `tomorrow`, `this morning`, `this afternoon`, `this evening`, `tonight`, and `now`.

Examples:

```bash
sivtr search terminal --status failure --latest 1 --json
sivtr s terminal -m "panic|failed" -v "example|sample" --since today --refs
sivtr s terminal -m "panic|failed" | sivtr filter @ -v "demo" -i title -f timeline
sivtr search agent --match "TODO|failed|next step" --since yesterday --format md
sivtr search pi --since today --sort oldest --format timeline
sivtr search pi/019e5941 --match "cargo test" --format compact
sivtr search terminal/session_13104/3/12 --format workset
```

## filter

```bash
sivtr filter [SOURCE] [OPTIONS]
```

Filters a source or piped WorkSet with the same shared WorkSet filter surface used by `search`. If `SOURCE` is omitted it defaults to `@`, meaning WorkSet JSON from stdin.

Options:

| Option | Meaning |
| --- | --- |
| `--parts` | Select matching part anchors instead of preserving the input anchor granularity |
| `--match <REGEX>`, `-m <REGEX>` | Case-insensitive content filter |
| `--exclude <REGEX>`, `-v <REGEX>` | Case-insensitive exclusion filter |
| `--in <FIELD>`, `-i <FIELD>` | `content`, `title`, `session`, `input`, `output`, `command`, or `all` |
| `--io <IO>` | With `--parts`, choose `all`, `input`, or `output` parts |
| `--kind <KIND>` | Part kind filter |
| `--status <STATUS>` | `success`, `failure`, or `unknown` |
| `--exit-code <CODE>` | Exact terminal process exit code |
| `--min-duration <DURATION>` | Minimum command duration |
| `--max-duration <DURATION>` | Maximum command duration |
| `--sort <SORT>` | `newest`, `oldest`, `duration`, `duration-asc`, `exit-code`, or `exit-code-asc` |
| `--cwd <PATH>` | Workspace directory used to resolve records |
| `--since <TIME>` / `--until <TIME>` / `--last <DURATION>` | Time filters |
| `--latest <N>` | Return the latest N matching anchors before final sort |
| `-l, --limit <N>` | Maximum result anchors to print |
| `--exclude-current`, `--other` | Exclude the current agent session from agent searches |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |
| `--save <NAME>` | Save the result WorkSet as `@name` |

Examples:

```bash
sivtr search terminal --json | sivtr filter @ -m error --refs
sivtr filter terminal --status failure --refs
sivtr filter @last --parts --io output --kind tool_output --refs
```

## var

```bash
sivtr var <COMMAND>
```

Manages named WorkSet variables.

| Command | Meaning |
| --- | --- |
| `set <name> [source]` | Save a source or piped WorkSet as `@name` |
| `list` | List saved variables with item counts and creation time |
| `rm <name>` | Remove one saved variable |
| `merge <name> <source>...` | Merge sources into a saved variable, deduplicating by anchor |
| `drop <name> <source>...` | Remove source anchors from a saved variable |
| `cleanup` | Remove all saved variables |

Examples:

```bash
sivtr var set ctx @last
sivtr filter terminal -m panic --json | sivtr var set failures
sivtr var list
sivtr var merge ctx @failures @last[1]
sivtr var drop ctx @noise
```

## nav

```bash
sivtr nav <SOURCE> <MOTION> [OPTIONS]
```

Moves WorkSet anchors deterministically through record/part/session structure. `nav` does not default-expand children; child movement must specify a 1-based index with `>N`.

Motion tokens compose left-to-right:

| Token | Meaning |
| --- | --- |
| `<` | Parent. Part/line to record; record to containing session records. |
| `>N` | Nth child, 1-based. Record children are its parts. |
| `+N` | Next sibling by N at the current level. |
| `-N` | Previous sibling by N at the current level. |
| `[A..B]` | Sibling window at the current level, relative to the current anchor. |
| `~` | Containing session records. |

Options:

| Option | Meaning |
| --- | --- |
| `--cwd <PATH>` | Workspace directory used to resolve records |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |

Examples:

```bash
sivtr nav @hit '<' --refs
sivtr nav @hit '>1' --refs
sivtr nav @hit '<+1>1' --refs
sivtr nav @hit '<[-2..+2]' --refs
sivtr nav @hit '~' --refs
```

Use `zoom` for simple neighboring record context. Use `nav` when the exact movement path matters.

## show

```bash
sivtr show <SOURCE> [OPTIONS]
```

Prints a workspace ref or WorkSet source such as `@last`, `@name`, or `@`.

Ref syntax:

```text
source/session[/dialogue[/line]]
```

Options:

| Option | Meaning |
| --- | --- |
| `--cwd <PATH>` | Workspace directory used to resolve sessions |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `--full` | Alias for `--format full` |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |

Examples:

```bash
sivtr show claude/<session-id>
sivtr show claude/<session-id>/3
sivtr show claude/<session-id>/3/7 --json
sivtr show terminal/current/2
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr show @last --full
sivtr show @ctx -f timeline
```

## serve

```bash
sivtr serve <COMMAND>
```

Manages the local remote-memory daemon. Share and remote commands auto-start it when needed.

| Command | Meaning |
| --- | --- |
| `start` | Start the daemon in the background |
| `stop` | Stop the running daemon cleanly |
| `restart` | Restart the daemon |
| `status` | Show daemon identity and runtime state |
| `logs` | Print the daemon log path |
| `foreground` | Run the daemon in the foreground |

```bash
sivtr serve start
sivtr serve status
sivtr serve logs
sivtr serve stop
```

## share

```bash
sivtr share [OPTIONS]
sivtr share <COMMAND>
```

Explicitly shares a local workspace for remote peers. Bare `sivtr share` is interactive: pick a workspace (Enter = current), ensure the share exists, and print a bare invite key on stdout.

Default interactive options:

| Option | Meaning |
| --- | --- |
| `--path <PATH>` | Workspace path; skips the picker after confirm |
| `--name <NAME>` | Stable share name; defaults to the workspace directory name |
| `--expires <DURATION>` | Invitation lifetime (`10m`, `2h`, `1d`); default `10m` |
| `--no-redact` | Disable secret redaction for this share |

Subcommands:

| Command | Meaning |
| --- | --- |
| `add [PATH] [--name NAME] [--no-redact]` | Expose a workspace through the daemon |
| `list` | List local shares |
| `remove <SHARE>` | Remove a share and all grants and invitations attached to it |
| `enable <SHARE>` / `disable <SHARE>` | Toggle a share without deleting it |
| `invite <SHARE> [--expires DURATION]` | Create a single-use invitation; prints the bare key on stdout |
| `grants <SHARE>` | List active peer grants for a share |
| `revoke <SHARE> <PEER>` | Revoke a peer's access to a share |

```bash
sivtr share
sivtr share add --name alice-desk
sivtr share invite alice-desk --expires 10m
sivtr share list
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
```

## remote

```bash
sivtr remote <COMMAND>
```

Mounts remote shares into the current git workspace as local aliases used in `origin:body` refs.

| Command | Meaning |
| --- | --- |
| `list` | List remote mounts in the current workspace |
| `add <ALIAS> <INVITE>` | Redeem an invitation and mount the remote share |
| `remove <ALIAS>` | Remove a local mount (grant remains until the owner revokes it) |
| `rename <ALIAS> <NEW>` | Rename a workspace-local mount |
| `test <ALIAS>` | Authenticated transport and authorization round trip |

```bash
sivtr remote add desk <invite-key>
sivtr remote test desk
sivtr remote list
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr show desk:agent/<session>/3 --full
sivtr remote rename desk bob-desk
sivtr remote remove desk
```

## peer

```bash
sivtr peer <COMMAND>
```

| Command | Meaning |
| --- | --- |
| `list` | List known peer identities |
| `forget <PEER>` | Forget a peer and remove all local mounts and grants involving it |

```bash
sivtr peer list
sivtr peer forget <peer>
```

## workspace

```bash
sivtr workspace [list]
sivtr wb list
```

Lists known local workspaces and their origin labels for `name:body` refs (for example `docs:codex/4`). Alias: `sivtr wb`.

```bash
sivtr wb list
```

Exact syntax for every remote subcommand is above. For the model, setup path, and safety defaults, see [Remote Access](/usage/remote-access/). For a teammate scenario, see [Remote collaboration memory](/playbooks/remote-collaboration-memory/).

## mcp

```bash
sivtr mcp serve
sivtr mcp install [OPTIONS]
sivtr mcp uninstall [OPTIONS]
sivtr mcp print-config <claude|cursor|codex>
```

Read-only MCP server for agent hosts, plus one-shot host registration.

### serve

Runs the MCP server on stdio:

```bash
sivtr mcp serve
```

Tools:

| Tool | Purpose |
| --- | --- |
| `sivtr_search` | Search terminal/agent memory; supports `desk:...` origins |
| `sivtr_show` | Expand a ref or WorkSet handle |
| `sivtr_zoom` | Neighboring record context |
| `sivtr_filter` | Narrow `@last` / `@name` / a source |
| `sivtr_status` | Version, hooks, providers, daemon, `wb` local origins, mounts, vars |

### install / uninstall

Writes or removes the sivtr MCP entry in agent host config (same idea as `codegraph install`):

```bash
sivtr mcp install -y                      # auto-detect hosts, global
sivtr mcp install -t claude,cursor -l global
sivtr mcp install -t claude -l local      # project .mcp.json
sivtr mcp uninstall -t all -y
```

| Flag | Meaning |
| --- | --- |
| `-t, --target` | `claude`, `cursor`, `codex`, `auto`, or `all` |
| `-l, --location` | `global` (default) or `local` |
| `-y, --yes` | Non-interactive |

Install locations:

| Target | Global path |
| --- | --- |
| Claude Code | `~/.claude.json` → `mcpServers.sivtr` |
| Cursor | `~/.cursor/mcp.json` → `mcpServers.sivtr` |
| Codex | `~/.codex/config.toml` → `[mcp_servers.sivtr]` |

Registered command is always:

```text
sivtr mcp serve
```

### print-config

Print a snippet without writing files:

```bash
sivtr mcp print-config claude
sivtr mcp print-config cursor
sivtr mcp print-config codex
```

MCP is not a full CLI mirror. Interactive, write, and capture commands stay on the CLI. Strategy still lives in the `sivtr-memory` skill.

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
