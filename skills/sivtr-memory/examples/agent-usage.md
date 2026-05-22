# Agent Usage Examples

These examples show how to turn vague user references into concrete `sivtr` retrieval steps.

## Example 1: Recent Failure

User:

```text
刚才那个报错修一下
```

Agent should not ask for pasted logs first. Search shared memory:

```bash
sivtr search "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --json --limit 20
```

If the project/tool is clear, narrow it:

```bash
sivtr search "cargo test|could not compile|error\\[E[0-9]+\\]|panicked" --json --limit 20
```

If snippets are insufficient, expand the latest command output:

```bash
sivtr copy out 1 --print
```

Then inspect relevant files, patch, and run verification.

## Example 2: Continue Work

User:

```text
继续
```

Agent should search for task continuity before guessing:

```bash
sivtr search "next step|TODO|blocked|decision|commit|test result|passed|failed" --json --limit 20
```

If one clear thread appears, summarize it and proceed. If multiple plausible threads appear, ask the user to choose.

Good response shape:

```text
I found two plausible continuations in sivtr:
1. Continue TUI lazy-load performance work.
2. Continue the sivtr-memory skill packaging.
Which one should I continue?
```

## Example 3: Prior Decision

User:

```text
之前为什么没做 infinite scroll？
```

Agent should search AI conversation history and decision terms:

```bash
sivtr search "infinite scroll|dynamic loading|viewport|decision|defer|先别改" --json --limit 20
```

Treat the result as decision context, not current truth. Quote narrowly and distinguish it from code state.

Good response shape:

```text
I searched sivtr for "infinite scroll|dynamic loading|viewport|decision|defer" and found prior AI-conversation context indicating it was intentionally deferred. That explains the decision history; I would still inspect the current code before making implementation claims.
```

## Example 4: Handoff

User:

```text
给下一个 agent 一个 handoff
```

Agent should collect evidence from both memory and recent terminal commands:

```bash
sivtr search "current goal|next step|TODO|blocked|decision|test result|commit" --json --limit 30
sivtr copy cmd 1..10 --print
```

Then produce:

- Goal
- Current state
- Changes made
- Validation run
- Decisions/deferred work
- Risks
- Next suggested steps

## Example 5: Missing Memory

User:

```text
上次部署失败是什么原因？
```

Agent searches:

```bash
sivtr search "deploy|deployment|failed|error|exit code|wrangler|vercel|cloudflare" --json --limit 20
```

If no relevant result appears, do not invent an answer.

Good response shape:

```text
I searched sivtr for deployment failure terms and found no matching local memory. I can either inspect the deployment config and try to reproduce the failure, or you can point me to the deploy command/service.
```

## Example 6: Permission Required

User:

```text
把之前那套配置清掉
```

Even if sivtr can find the relevant history, destructive/config-changing actions need confirmation.

Agent may search for context:

```bash
sivtr search "config|configuration|init|shell|clear|remove" --json --limit 20
```

But before deleting or modifying config, ask for explicit confirmation with the exact target.
