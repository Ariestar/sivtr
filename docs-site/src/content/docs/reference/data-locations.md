---
title: Data Locations
description: Where sivtr stores configuration, history, session logs, and provider data.
---

`sivtr` is local-first. Most data it uses is already on your machine, and everything sivtr generates lives under one home directory:

- `SIVTR_HOME` if set (Grok-style whole-home relocation),
- else `~/.sivtr` on every platform.

```text
~/.sivtr/
  config.toml               ← configuration
  history.db                ← captured terminal output (SQLite)
  sets/                     ← WorkSet checkpoints (@last, @name)
  workspaces/<key>/terminals/  ← shell session logs (session_<pid>.jsonl)
  cache/                    ← agent session parse cache
  identity.key              ← device identity for remote memory
  remote-state.db           ← peers, shares, grants, invites, mounts
  daemon.json / daemon.lock / daemon.log
```

Use CLI commands instead of editing these files directly.

## Migrating from earlier versions

Sivtr ≤ 0.4 scattered its data across the platform config, state, and data directories. `sivtr doctor` reports anything still there, and `sivtr doctor --fix` moves it into the single home. Migration merges directories and never overwrites an existing destination; conflicting items are reported and left in place.

## Config file

```text
<SIVTR_HOME or ~/.sivtr>/config.toml
```

## Shell session logs

Shell integration writes per-process structured session logs under `<home>/workspaces/<key>/terminals/session_<pid>.jsonl`.

These logs power:

- `sivtr import`;
- `sivtr copy` command-block workflows;
- `sivtr diff`;
- command-block navigation in the browser.

## History database

Captured terminal output is stored in a local SQLite history database when `[history].auto_save = true`:

```text
<home>/history.db
```

Use CLI commands instead of editing the database directly:

```bash
sivtr history list
sivtr history search "panic"
sivtr history show 42
```

Retention is controlled by:

```toml
[history]
max_entries = 0
```

`0` means unlimited.

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

## Codex exported mirrors

`codex export` writes a copy of local Codex session files into a destination you choose:

```bash
sivtr codex export --dest /srv/sivtr/root-codex
```

The destination receives a `sessions/` tree. Another account can read it by adding:

```toml
[codex]
session_dirs = ["/srv/sivtr/root-codex/sessions"]
```

Use read-only permissions for shared mirrors when possible.

## Generated launchers

Linux shortcut generation writes:

- `~/.local/bin/sivtr-pick-codex`;
- `~/.local/share/applications/sivtr-pick-codex.desktop`.

macOS shortcut generation writes:

- `~/.local/bin/sivtr-pick-codex`;
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`.

Windows hotkey state is stored under the single home and is managed by:

```bash
sivtr hotkey status
sivtr hotkey stop
```

## Remote daemon state

Cross-device remote memory uses a device-scoped daemon. State lives under the single home:

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
