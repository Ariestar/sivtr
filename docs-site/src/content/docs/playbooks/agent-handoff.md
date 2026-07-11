---
title: Agent Handoff
description: Prepare a concise evidence-backed handoff for the next human or agent.
---

## Demo

<img src="/demo/6.gif" alt="Prepare an evidence-backed agent handoff with sivtr" class="demo-wide" />

## The scenario

You are done for the day, or switching to a different agent. You want the next person or agent to pick up exactly where you left off — with goals, decisions, validation results, and next steps all documented.

## What you say

```text
Give the next agent a handoff.
```

## How it works

The agent collects recent decisions and validation evidence:

```bash
sivtr search agent --match "current goal|next step|TODO|blocked|decision|test result|commit|passed|failed" --since today --format json --limit 30
sivtr search terminal --since today --sort oldest --format timeline --limit 20
sivtr copy cmd 1..10 --print
```

Then it produces a structured handoff document.

## Example output

```text
## Goal
Migrate docs-site to Bun-only and add skills documentation.

## Current state
Both tasks complete. Build passes, 69 pages generated.

## Validation
- `bun run check`: 0 errors, 0 warnings, 0 hints.
- `bun run build`: 69 pages built successfully.
- VS Code extension: 8 tests pass, vsix packaged.

## Decisions made
- Playbooks moved to a separate top-level section for future video demos.
- Remote collaboration uses explicit share + mount; handoffs stay local by default.

## Risks
- Sitemap warning (non-blocking, needs `site` config in astro.config.mjs).

## Next steps
- Record demo videos for playbook pages.
- Commit and push all documentation changes.
```

## Demo video outline

This playbook works well in longer videos or live streams where work continues across multiple agent sessions.
