---
title: Publish a read-only browser conversation link
description: Turn a local agent conversation into an encrypted, browser-readable snapshot link.
---

`sivtr publish` turns a local agent conversation into a browser link. Viewers do not install Sivtr or sign in. The link still works after your machine is offline.

The result is a one-shot snapshot, not a live share. Later edits in the original session do not update an existing link; create a new one.

## Three things to remember

1. Quote WorkSet names in PowerShell: `'@share_ready'`.
2. v1 publishes consecutive turns from one local agent session. Search keeps the 5 newest hits by default; publish sorts those records by index, but they still have to be adjacent in the session.
3. The full URL is the credential. Anyone who has it can read the snapshot.

## Configure the endpoint first

`[publish].endpoint` defaults to empty. Set it to the publication service you actually run — a self-hosted URL such as `https://share.hnnulwh.cn`, or a Cloudflare Worker hostname if that network can reach Cloudflare. The CLI does not fail over between backends.

```toml
[publish]
endpoint = "https://share.hnnulwh.cn"
```

## Check the CLI

```powershell
sivtr --version
sivtr publish --help
```

If you see `unrecognized subcommand 'publish'`, the binary on `PATH` is too old.

## Build a WorkSet

```powershell
sivtr search codex/<session-id> --sort oldest --latest 50 --save share_ready --refs
```

`--latest 50` takes the 50 most recent turns in that session (search defaults to 5 when neither `--latest` nor `--limit` is set). `--sort oldest` stores them chronologically for reading; `publish` also sorts by record index before checking continuity.

In PowerShell the saved set is `'@share_ready'`.

Do not publish a mixed `@last`, terminal records, remotes, or a BM25 hit list that skipped turns.

## Preview locally

Preview never uploads:

```powershell
sivtr publish preview '@share_ready' --format human
```

Tokens, private keys, Bearer values, and secret assignments become `[REDACTED]`. Absolute paths, emails, and internal URLs are warnings only.

## Create the link

```powershell
sivtr publish create '@share_ready' --expires 7d --yes
```

Allowed lifetimes: `1d`, `7d` (default), `30d`, `90d`. There is no permanent link.

If the preview still has path, email, or internal-URL warnings, **`--allow-warnings` is required even in an interactive terminal**:

```powershell
sivtr publish create '@share_ready' --expires 7d --yes --allow-warnings
```

On success, stdout is only the URL. The host comes from `[publish].endpoint`. The decryption key is the `#k=...` fragment and is not sent to the server.

## List, reprint, revoke

```powershell
sivtr publish list
sivtr publish link 7d_xxxxxxxxxxxxxxxxxxxxxx
sivtr publish revoke 7d_xxxxxxxxxxxxxxxxxxxxxx --yes
```

Management tokens live only in local `publication-state.db`. If that database is lost, v1 cannot recover revoke rights.

## `publish` vs `share`

| | `publish` | `share` |
| --- | --- | --- |
| Result | Immutable browser snapshot | Live workspace mount |
| Viewer | No Sivtr, no login | Usually Sivtr/daemon + grant |
| Publisher online? | No | Usually yes |
| Server sees | Ciphertext only | Records over the remote protocol |

v1 does not publish terminal records, remotes, mixed sessions, tool/thinking/skill parts, refs, `cwd`, or attachments.

See also the [CLI reference](/reference/cli/) and [configuration](/usage/configuration/).
