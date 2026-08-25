---
title: Publish a read-only browser conversation link
description: Turn a local agent conversation into an encrypted, browser-readable snapshot link.
---

`sivtr publish` turns a local agent conversation into a browser link. Viewers do not install Sivtr or sign in. The link still works after your machine is offline.

The result is a one-shot snapshot, not a live share. Later edits in the original session do not update an existing link; create a new one.

## Three things to remember

1. Quote WorkSet names in PowerShell: `'@share_ready'`.
2. There are two selection modes: a whole-record WorkSet produces schema v1 with consecutive User/Assistant turns; `--pick` produces schema v2 with arbitrary atoms and non-contiguous parts from one local agent session.
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

### Pick atomic content

Use `--pick` when a complete turn is too broad, or when the public snapshot should include selected tool, skill, or thinking atoms:

```powershell
# Preview a selection without saving it
sivtr publish preview codex/<session-id> --pick --format human

# Save the exact part anchors for reuse
sivtr publish preview codex/<session-id> --pick --save share_ready

# Preview and create from the same selection
sivtr publish preview '@share_ready' --format human
sivtr publish create '@share_ready' --expires 7d --yes
```

The picker accepts exactly one local agent session. It supports whole-dialogue selection, marked content blocks, cross-page selection, and non-contiguous turns. `Space` marks a dialogue or content block, `v` selects a block range, `J`/`K` moves between selected dialogues, and `Tab` switches Input/Output. Press `Enter` on Dialogues to submit whole-dialogue selection; press `y` in Content to submit the current or marked blocks; `Enter` in Content folds or expands the focused block. A character range that does not identify a complete block is rejected instead of being widened to a whole turn.

User, Assistant, Skill, and Thinking are separate atoms. A ToolCall and its ToolResult form one inseparable Tool atom; selecting either side expands to both. The saved WorkSet contains only the selected local anchors and the records those anchors belong to; unselected turns are not stored. The public snapshot does not contain WorkRef, session IDs, record/part numbers, paths, or `cwd`.

Whole-record WorkSets remain schema v1 and require adjacent record indices. Part-anchor WorkSets are schema v2, allow gaps, and show “部分内容未分享” (some content was not shared) between separated selections. Whole and part anchors cannot be mixed.

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

Both modes reject terminal records, remotes/groups, mixed providers or sessions, WorkRefs, `cwd`, session paths, provider envelopes, and attachments. v1 projects only consecutive User/Assistant turns; v2 can project User, Assistant, Tool, Skill, and Thinking atoms, with ToolCall and ToolResult kept together.

See also the [CLI reference](/reference/cli/) and [configuration](/usage/configuration/).
