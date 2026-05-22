---
name: sivtr-memory
description: Use when the agent needs shared local work memory: terminal commands/output, AI conversation history, recent failures, prior decisions, handoff context, recap context, or validation evidence. Search sivtr before asking the user to paste logs or repeat what happened earlier.
---

# Sivtr Memory

Sivtr gives you searchable access to the shared local work memory of this machine: terminal commands, terminal output, and AI conversation history from humans and other agents.

Use it as your memory interface before asking the user to paste logs, repeat prior decisions, or explain what happened earlier.

This memory is evidence, not omniscience. It may be incomplete, stale, or wrong. Search it, quote it narrowly, and verify current code or command state before making claims.

Core rule:

> Search for evidence first. Expand only the smallest relevant context. Ask when memory is missing, ambiguous, stale, or permission is required.

This rule applies to evidence retrieval, not to product clarification or safety approval. If the missing information is user intent or permission, ask immediately.

## When to Use

Use this skill when the user says or implies any of these:

- "刚才报错了", "the last command failed", "look at the error"
- "继续", "continue", "接着做", "resume from earlier"
- "上次怎么说的", "what did we decide before", "why did we choose X"
- "找一下那个报错", "search previous output", "find the failure"
- "handoff", "summarize what happened", "write a recap"
- You need evidence for a claim about recent build/test/lint/deploy output
- You need prior AI discussion, project decisions, rejected approaches, or debugging trails

Also use it proactively during debugging when command output is missing, truncated, or too large for the current context.

## First Check

Before relying on sivtr in a new environment, run:

```bash
sivtr --version
```

If that fails, the skill cannot retrieve shared memory. Continue with normal shell/file inspection and mention the missing CLI only if it affects the task.

## Default Retrieval Workflow

1. Convert the user's vague reference into a query.
2. Search with a small limit and JSON output.
3. Inspect result source/session/dialogue/snippet.
4. If the snippet is enough, answer with evidence.
5. If more context is needed, expand with `sivtr copy ... --print` or a narrower second search.
6. Ask the user only after local memory has been checked and still lacks the needed fact.

## Non-Interactive Safety Rules

- Prefer non-interactive commands: `sivtr search ... --json`, `sivtr copy ... --print`.
- Do not open TUI pickers (`--pick`, hotkey picker) unless the user explicitly wants interactive selection.
- Do not run `sivtr clear`, hotkey start/stop, shell init, or config mutation unless the user explicitly asks.
- `sivtr copy` can affect the clipboard unless `--print` is used. Agents should use `--print` by default.
- Avoid dumping huge histories into the model. Search narrowly first, then expand only the relevant block/dialogue.
- If `sivtr` is not installed or no session log exists, say so briefly and continue with normal tools. Do not invent memory results.

## Load References as Needed

References are relative to this skill directory.

- `references/memory-model.md` — product mental model, use cases, and boundaries.
- `references/commands.md` — search and expansion command cookbook.
- `references/workflows.md` — debugging, continuation, handoff, and recap workflows.
- `references/output-discipline.md` — how to report evidence and avoid anti-patterns.
- `examples/agent-usage.md` — concrete examples that map user requests to sivtr commands and response shapes.

Read only the file needed for the current task. For example, debugging a recent failure usually needs `references/commands.md` and `references/workflows.md`; answering "what is sivtr memory?" usually needs `references/memory-model.md`; learning how to respond to vague user requests usually needs `examples/agent-usage.md`.
