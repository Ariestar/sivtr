---
title: Remote Access
description: Share a workspace read-only and mount another device's memory as a local origin alias.
---

Cross-device memory lets two machines running `sivtr` read each other's workspace sessions like local sources. Sharing is explicit, read-only, and redacted by default.

If you want the teammate scenario first, see [Remote collaboration memory](/playbooks/remote-collaboration-memory/). This page is the feature guide.

## Model

| Piece | Meaning |
| --- | --- |
| Device daemon | One per machine. Started by `sivtr serve`, auto-started by share/remote commands when needed. |
| Share | An explicitly exposed local workspace. |
| Invite | Single-use key with a short TTL. Printed as a bare key on stdout. |
| Grant | Peer permission after redeeming an invite. |
| Mount | Workspace-local alias that becomes the left side of `origin:body` refs. |

Refs use one form:

```text
codex/4                 # local current workspace
docs:codex/4            # another local workspace by name
desk:terminal/...       # mounted remote alias
```

List local origin labels with `sivtr wb list`. Unregistered origins error.

## Owner setup

On the machine that owns the workspace:

```bash
sivtr share                   # pick workspace (Enter = current), print invite key
```

Non-interactive:

```bash
sivtr share add --name alice-desk
sivtr share invite alice-desk --expires 10m
```

Useful owner commands:

```bash
sivtr share list
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
sivtr share disable alice-desk
sivtr share remove alice-desk
sivtr serve status
```

## Peer setup

In the git workspace where you want the mount:

```bash
sivtr remote add desk <invite-key>
sivtr remote test desk
sivtr remote list
```

Useful peer commands:

```bash
sivtr remote rename desk bob-desk
sivtr remote remove desk          # local mount only; grant remains until owner revokes
sivtr peer list
sivtr peer forget <peer>
```

## Use remote memory

Mounted origins work with the same WorkSet surface as local sources:

```bash
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr s desk:agent -m "panic|failed|decision" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr zoom desk:agent/<session>/3 -C 2 --save remote_ctx --refs
sivtr filter @remote_hits -m "cargo test" --save remote_tests --refs
sivtr nav @remote_tests[1] '<[-1..+1]' --refs
sivtr copy ref desk:terminal/session_42/3/o/1 --print
```

## Safety defaults

- Nothing is shared until `sivtr share` or `share add` runs.
- Access is read-only. Peers cannot write sessions or run commands on the owner machine.
- Secret redaction is on by default (`--no-redact` disables it for a share).
- Invites are single-use and short-lived (default `10m`).
- Transport between daemons is encrypted iroh.
- Local-first remains default: unknown origins fail instead of probing the network.

## Daemon and data

```bash
sivtr serve start
sivtr serve status
sivtr serve logs
sivtr serve stop
```

State lives under `data_dir()` (`SIVTR_DATA_DIR` override, else the platform config dir under `sivtr`):

| File | Purpose |
| --- | --- |
| `identity.key` | Stable device identity |
| `remote-state.db` | Peers, shares, grants, invites, mounts |
| `daemon.json` / `daemon.lock` / `daemon.log` | Runtime control and logs |

See [Data Locations](/reference/data-locations/) and [Local-first and Privacy](/explanation/local-first-privacy/).

## Command map

| Command | Purpose |
| --- | --- |
| `sivtr share` | Interactive share + invite |
| `sivtr share add\|list\|invite\|grants\|revoke...` | Manage shares |
| `sivtr remote add\|list\|remove\|rename\|test` | Manage mounts in the current workspace |
| `sivtr peer list\|forget` | Manage known peer identities |
| `sivtr serve ...` | Manage the device daemon |
| `sivtr wb list` | List local workspace origin labels |

Exact syntax: [CLI Reference](/reference/cli/).
