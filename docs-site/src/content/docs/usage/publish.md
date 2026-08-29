---
title: Publish a read-only browser conversation link
description: Turn a local agent conversation into an encrypted, browser-readable snapshot link.
---

`sivtr publish` turns a local agent conversation into a browser link. Viewers do not install Sivtr or sign in. The link still works after your machine is offline.

The result is a one-shot snapshot, not a live share. Later edits in the original session do not update an existing link; create a new one.

## Three things to remember

1. A saved WorkSet name can be passed directly, without `@`: `sivtr publish share_ready`.
2. `sivtr publish preview` opens the existing TUI; the same WorkSet selection drives preview and publish.
3. The full URL is the credential. Anyone who has it can read the snapshot.

## Configure the endpoint first

`[publish].endpoint` defaults to `https://share.hnnulwh.cn`. Change it in `config.toml` if you self-host or use another compatible service. The CLI does not fail over between backends.

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

Publish it with `sivtr publish share_ready`; the saved WorkSet remains reusable by
the other WorkSet commands.

Do not publish a mixed `@last`, terminal records, remotes, or a BM25 hit list that skipped turns.

## Preview locally

Preview never uploads:

```powershell
sivtr publish preview share_ready --format human
```

Tokens, private keys, Bearer values, and secret assignments become `[REDACTED]`. Absolute paths, emails, and internal URLs are warnings only.

### Pick atomic content

Run preview without a source to choose content in the existing workspace TUI:

```powershell
sivtr publish preview
```

The picker accepts exactly one local agent session for publication. It supports whole-dialogue selection, marked content blocks, cross-page selection, and non-contiguous turns. `Space` marks a dialogue or content block, `v` selects a block range, and `Tab` switches Input/Output. Press `Enter` on Dialogues or `y` in Content to return the selected WorkSet for preview. Press `p` to open the publication lifetime overlay; the bare workspace creates the link after confirmation, while `publish preview` prints the selected snapshot locally. A character range that does not identify a complete block is rejected instead of being widened to a whole turn.

User, Assistant, Skill, and Thinking are separate atoms. A ToolCall and its ToolResult form one inseparable Tool atom; selecting either side expands to both. The selected WorkSet is held for this preview unless a separate WorkSet command saved it explicitly or a name was entered in the publication panel. The public snapshot does not contain WorkRef, session IDs, record/part numbers, paths, or `cwd`.

Whole-record WorkSets remain schema v1 and require adjacent record indices. Part-anchor WorkSets are schema v2, allow gaps, and show “部分内容未分享” (some content was not shared) between separated selections. Whole and part anchors cannot be mixed.

## Create the link

```powershell
sivtr publish share_ready --expires 7d --yes
```

Allowed lifetimes: `2h`, `1d`, `3d`, `7d` (default), `30d`. There is no permanent link.

If the preview still has path, email, or internal-URL warnings, **`--allow-warnings` is required even in an interactive terminal**:

```powershell
sivtr publish share_ready --expires 7d --yes --allow-warnings
```

The TUI `p` flow uses the same warning rule and asks for an explicit privacy confirmation after the picker.

On success, stdout is only the URL. The host comes from `[publish].endpoint`. The decryption key is the `#k=...` fragment and is not sent to the server.

## List, reprint, revoke

```powershell
sivtr publish list
sivtr publish link
sivtr publish revoke
```

In an interactive terminal, `list` shows clickable active links. `link` and `revoke`
choose a publication when no ID is supplied; `--yes` only skips the revoke
confirmation and is required for non-interactive revocation. Management tokens
live only in local `publication-state.db`; this applies to both v1 and v2, so
losing that database means the management token cannot be recovered for revoke.

## `publish` vs `share`

| | `publish` | `share` |
| --- | --- | --- |
| Result | Immutable browser snapshot | Live workspace mount |
| Viewer | No Sivtr, no login | Usually Sivtr/daemon + grant |
| Publisher online? | No | Usually yes |
| Server sees | Ciphertext only | Records over the remote protocol |

Both modes reject terminal records, remotes/groups, mixed providers or sessions, WorkRefs, `cwd`, session paths, provider envelopes, and attachments. v1 projects only consecutive User/Assistant turns; v2 can project User, Assistant, Tool, Skill, and Thinking atoms, with ToolCall and ToolResult kept together.

See also the [CLI reference](/reference/cli/) and [configuration](/usage/configuration/).
