---
title: MCP Cross-Session Recovery
description: Resume the right task through MCP search, a named WorkSet, and cited evidence.
---

## The scenario

You open a fresh agent session and say "continue retry-policy." A newer record belongs to another task, and an older retry-policy record proposed a different cap. Taking the latest record would resume the wrong thread.

## What you say

```text
Continue the retry-policy work. Don't use the latest record; cite the saved decision and checks.
```

## How it works

The agent uses `sivtr mcp serve` tools, not the CLI, and does not treat `@last` as the current task:

1. `sivtr_search` with a task filter (`query` / `match_regex`) so the unrelated newer record is out.
2. `sivtr_filter` (or `save`) to keep the current thread and store it as a named WorkSet such as `@current`. The next search overwrites `@last`.
3. After the MCP process restarts (new session or idle-exit), `sivtr_show` expands `@current[n]`, not `@last`.

## Example tool arguments

```text
sivtr_search  source=@fixture  query=retry-policy  match_regex=retry-policy  save=recovery
sivtr_filter  source=@recovery  since=<current thread window>  save=current
sivtr_show    source=@current[1]  mode=full
```

WorkSet indexes such as `@current[1]` are 1-based for that saved selection. Compare each expanded `contents[].ref` with the returned anchor before using its text.

## Why not latest

| Record | Time (UTC) | Role |
| --- | --- | --- |
| `codex/synthetic-retry-policy-old/1` | 2026-01-01 10:00 | Stale failed check; proposed cap 5. |
| `codex/synthetic-retry-policy/1` | 2026-01-02 10:00 | Decision: cap 2, keep exponential backoff. |
| `codex/synthetic-retry-policy/2` | 2026-01-02 11:00 | Check output: 3 passed; 0 failed. |
| `codex/synthetic-retry-policy/3` | 2026-01-02 12:00 | Next step: add a cancellation case. |
| `codex/synthetic-color-theme/1` | 2026-01-03 10:00 | Unrelated newer task, marked complete. |

A recovery note cites the three current records. The color-theme row and the old cap-5 proposal are not the thread to resume.

This path is covered by `tests/mcp_session_recovery.rs` with a synthetic WorkSet under `SIVTR_HOME`. That fixture is not evidence that the invented commands passed on this checkout.

See [MCP reference](/reference/cli/#mcp) and [Agent handoff](/playbooks/agent-handoff/).
