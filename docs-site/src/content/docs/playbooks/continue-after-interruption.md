---
title: Continue After Interruption
description: Let the agent search local memory to continue the right thread.
---

## Demo

<img src="/demo/5.gif" alt="Continue work after interruption using sivtr memory" class="demo-wide" />

## The scenario

Your session was interrupted, compacted, or you switched context. You come back and say "continue." With `sivtr`, the agent can search local memory for the most recent work thread.

## What you say

```text
Continue.
```

## How it works

The agent searches for recent goals, decisions, and validation markers:

```bash
sivtr search agent --match "next step|TODO|blocked|decision|test result|passed|failed" --since today --format json --limit 20
sivtr search terminal --since today --sort oldest --format timeline --limit 20
```

If one clear thread emerges, it continues. If multiple threads are plausible, it asks you to choose.

## Example interaction

```text
User: Continue.

Agent: I found two recent threads in sivtr memory:
1. Skills documentation (last activity: added playbooks section, build passed).
2. VS Code extension packaging (last activity: tests passed, vsix built).
Which one should I continue?
```

## Demo video outline

1. Work on a task until some progress is made.
2. Start a fresh agent session (simulating interruption).
3. Say only "continue."
4. Show the agent reconstructing context from memory.
