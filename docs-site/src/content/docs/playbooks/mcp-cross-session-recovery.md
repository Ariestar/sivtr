---
title: MCP Cross-Session Recovery
description: Recover a synthetic task through real MCP calls, saved selections, and exact evidence references.
---

## The scenario

An agent needs to resume the `retry-policy` task in a fresh session. A newer record belongs to another task, and an older `retry-policy` record reports a failed check and a different retry proposal. Choosing the latest record alone would produce the wrong handoff.

This example uses a fixed, entirely synthetic [WorkSet fixture](https://github.com/Ariestar/sivtr/blob/main/tests/fixtures/mcp-session-recovery.json). It contains the target task's decision, validation record, and next step, plus the older record and newer unrelated record. Its text describes invented work; it contains no private transcript or real validation result.

| Record | Time (UTC) | Evidence |
| --- | --- | --- |
| `codex/synthetic-retry-policy-old/1` | January 1, 2026, 10:00 | Old failed verification; proposed retry cap of 5. |
| `codex/synthetic-retry-policy/1` | January 2, 2026, 10:00 | Decision: cap retries at 2; keep exponential backoff. |
| `codex/synthetic-retry-policy/2` | January 2, 2026, 11:00 | Synthetic test output: 3 passed; 0 failed. |
| `codex/synthetic-retry-policy/3` | January 2, 2026, 12:00 | Next step: add a cancellation case. |
| `codex/synthetic-color-theme/1` | January 3, 2026, 10:00 | Unrelated color-theme task marked complete. |

## Run the example

From a source checkout with the [development requirements](https://github.com/Ariestar/sivtr/blob/main/CONTRIBUTING.md) installed:

```bash
cargo test --locked --test mcp_session_recovery -- --nocapture
```

The [integration check](https://github.com/Ariestar/sivtr/blob/main/tests/mcp_session_recovery.rs) starts the compiled `sivtr mcp serve --idle-exit 0` process and exchanges JSON-RPC messages over stdio. It creates a temporary data directory, saves the materialized fixture as `@fixture`, and queries only that WorkSet. It sets `SIVTR_DATA_DIR` for the server, placing the fixture and saved selections under `SIVTR_DATA_DIR/sets`; without that override, the usual platform storage location is unchanged. It does not discover or import native provider sessions. The server still reads the normal sivtr configuration if present, which must be valid.

The check verifies returned refs, source fields, selected records, and expanded content. It also restarts the server before using the saved selection, so recovery does not depend on one live MCP process.

## Search and narrow the evidence

After the MCP initialization handshake and `tools/list`, call `sivtr_search` with a task-specific content filter:

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "sivtr_search",
    "arguments": {
      "source": "@fixture",
      "query": "retry-policy",
      "match_regex": "retry-policy",
      "limit": 10,
      "save": "recovery",
      "detail": "timeline"
    }
  }
}
```

`query` ranks results; `match_regex` bounds the matching set. This search returns four anchors, with `source: "codex"` in each timeline item. It excludes the newer unrelated record but still matches the older record. Save this result as `@recovery`, then narrow it to the fixture's current work period:

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "sivtr_filter",
    "arguments": {
      "source": "@recovery",
      "since": "2026-01-02T00:00:00Z",
      "save": "current",
      "detail": "workset"
    }
  }
}
```

The date is part of this fixed scenario. For real work, establish the intended task and relevant time range from the user's request and available evidence; a newer timestamp alone does not establish that a decision supersedes another one.

MCP results place JSON in a text block under `result.content`. Parse that block's `text`; do not expect a `structuredContent` field. Search returns `anchors` and, in timeline mode, `items` with `ref`, `source`, `time`, and `snippet`. The filtered result contains `anchors` and the materialized `workset` because this call requested `detail: "workset"`.

The decoded filter result contains the following fields. Only its `workset` field is omitted here; the test checks that its records back these same anchors:

```json
{
  "count": 3,
  "anchors": [
    "codex/synthetic-retry-policy/3",
    "codex/synthetic-retry-policy/2",
    "codex/synthetic-retry-policy/1"
  ],
  "saved_as": "current"
}
```

## Resume from a saved selection

`sivtr_search` and `sivtr_filter` update `@last`. The check deliberately searches the unrelated task after saving `@current`, then stops and starts the MCP server. The next session expands `@current` with `sivtr_show` instead of assuming `@last` still contains the intended task. Saving another result with `save: "current"` would overwrite the named selection.

Use the `anchors` returned by the filtered result to choose evidence. WorkSet selectors such as `@current[1]` are 1-based; their position belongs to that saved selection. Compare each expanded `contents[].ref` with the corresponding returned anchor before using its text.

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "sivtr_show",
    "arguments": {
      "source": "@current[3]",
      "mode": "full"
    }
  }
}
```

The decoded `sivtr_show` result is reproduced in full below. The JSON-RPC envelope and text-block wrapper are omitted:

```json
{
  "count": 1,
  "anchors": ["codex/synthetic-retry-policy/1"],
  "contents": [
    {
      "ref": "codex/synthetic-retry-policy/1",
      "content": "SYNTHETIC retry-policy decision: cap retries at 2; keep exponential backoff."
    }
  ]
}
```

A WorkRef identifies the evidence location. A saved WorkSet preserves selected anchors and their materialized records across server restarts while that saved set remains available. MCP accepts these handles directly; the CLI's stdin `@` pipeline is not an MCP transport. The check also expands `@current[2]` for validation and `@current[1]` for the next step.

## Write the recovery note

After expanding all three selected records, an evidence-backed recovery note can say:

- **Recorded decision:** on January 2, cap retries at 2 and retain exponential backoff. Source: `codex/synthetic-retry-policy/1`.
- **Recorded validation:** the synthetic `cargo test retry_policy --locked` output says "3 passed; 0 failed." Source: `codex/synthetic-retry-policy/2`. This is saved fixture content, not a test just run against the current checkout.
- **Recorded next step:** add a cancellation case before considering the task complete. Source: `codex/synthetic-retry-policy/3`.
- **Agent inference:** resume with cancellation coverage, then inspect the current files and run the relevant checks. The task is not established as complete by the unrelated color-theme record or by the historical passing output.

The automated check proves this fixture's stdio request path, selection persistence, exclusion of unrelated and stale records, and evidence expansion. It does not evaluate native provider ingestion, an agent's reasoning quality, human time savings, or retrieval generalization. The fixture's reported validation is synthetic; only the integration check itself is executed here.

See [MCP reference](/reference/cli/#mcp) for server setup and [Agent handoff](/playbooks/agent-handoff/) for the broader handoff workflow.
