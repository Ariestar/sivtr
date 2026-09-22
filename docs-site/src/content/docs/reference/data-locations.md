---
title: Data Locations
description: Where sivtr stores configuration, the unified archive, session logs, and provider data.
---

`sivtr` is local-first. Most data it uses is already on your machine. Generated data lives under one home: `SIVTR_HOME` if set, else `~/.sivtr` on every platform.

```text
<SIVTR_HOME or ~/.sivtr>/
  config.toml
  identity.key
  remote-state.db
  publication-state.db
  sets/                      # named WorkSets (@last, @name)
  workspaces/                # terminal session logs
  cache/
    archive.db               # search index; also holds one-shot pipe/run captures
    bm25-*.bin               # safe to delete
  daemon.json / daemon.lock / daemon.log
```

`sivtr doctor --fix` migrates leftover files from the old platform config/state directories into this home. Do not delete `workspaces/`, `sets/`, or `identity.key`. Deleting `bm25-*.bin` only forces a rebuild. `archive.db` also stores one-shot `pipe`/`run` captures that are not in `workspaces/`.

## Config file

| Platform | Path |
| --- | --- |
| All | `~/.sivtr/config.toml` (`SIVTR_HOME` override) |

## Shell session logs

After installing shell integration with `sivtr init` or `sivtr setup` and restarting, the shell runs inside
`sivtr pty-proxy run` and every finished command appends one structured entry to a
per-terminal log:

| Typical path |
| --- |
| `<home>/workspaces/<workspace-key>/terminals/<terminal_id>.jsonl` |

`<workspace-key>` identifies the git repository — every worktree of one repository shares it —
and `<terminal_id>` identifies the shell session, so terminals never mix. Nothing is recorded
outside a git repository.

These logs power:

- the `sivtr` workspace browser;
- `sivtr copy` command-block workflows;
- `sivtr diff`;
- command-block navigation in the browser.

`sivtr clear` removes the current terminal's log; `sivtr clear --all` removes every workspace
session tree.

## Agent provider data

`sivtr` reads provider-owned local data. It does not upload transcripts.

| Provider | Data source |
| --- | --- |
| Codex | `~/.codex/sessions` rollout JSONL files |
| Claude Code | Current transcript/session environment and local Claude transcripts |
| Hermes | Primary: `$HERMES_HOME/state.db` (Windows default `%LOCALAPPDATA%\hermes`, else `~/.hermes`). Residual: `sessions/*.jsonl` under the same home. |
| OpenCode | OpenCode local database |
| Cursor | `~/.cursor/projects/**/agent-transcripts/**/*.jsonl` (override home with `CURSOR_HOME`) |
| OpenClaw | `~/.openclaw/agents/<id>/agent/openclaw-agent.sqlite` (legacy JSONL under `sessions/`) |
| Grok | `~/.grok/sessions/**` (`summary.json` + `chat_history.jsonl`; override home with `GROK_HOME`) |
| Pi | Pi agent session JSONL files |

Provider formats differ; `sivtr` normalizes them into sessions and dialogue units for copy, picker, search, and show workflows.

## Unified archive

Search, show, copy, picker, TUI, and MCP queries read from a unified local archive instead of parsing native session files on every query.

| Path |
| --- |
| `<home>/cache/archive.db` |

It is a SQLite database (WAL mode) written by the sync engine: `sivtr sync` runs a pass explicitly, queries run an automatic freshness pass when the archive is older than `[sync].max_age_secs`, and `pipe`/`run` write one-shot terminal captures directly to it. Native agent session files and shell session logs remain the source of truth: the sync engine reads them, and session-addressed loads self-heal by parsing the native file when the archive copy is missing or stale. BM25 files under `cache/` can be deleted and rebuilt. Do not delete `archive.db` if you need one-shot `pipe`/`run` captures.
## Generated launchers

Linux shortcut generation writes:

- `~/.local/bin/sivtr-pick-codex`;
- `~/.local/share/applications/sivtr-pick-codex.desktop`.

macOS shortcut generation writes:

- `~/.local/bin/sivtr-pick-codex`;
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`.

Windows hotkey state is stored under `sivtr`'s platform config/state area and is managed by:

```bash
sivtr hotkey status
sivtr hotkey stop
```

## Remote daemon state

Cross-device remote memory uses a device-scoped daemon. Files live under the single home (`SIVTR_HOME` / `~/.sivtr`).

| File | Purpose |
| --- | --- |
| `identity.key` | Stable device identity for iroh |
| `remote-state.db` | SQLite peers, shares, grants, invites, mounts, audit |
| `daemon.json` | Running daemon control info (port, token, node id) |
| `daemon.lock` | Single-instance lock |
| `daemon.log` | Daemon log file (`sivtr serve logs`) |

```bash
sivtr serve status
sivtr serve logs
sivtr share list
sivtr remote list
sivtr peer list
sivtr ws list
```

Remote access is opt-in. Nothing is shared until `sivtr share` (or `share add`) runs. Mounts are workspace-local aliases registered with `sivtr remote add`. Feature guide: [Remote Access](/usage/remote-access/).
