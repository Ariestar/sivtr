---
name: sivtr-memory
description: "Use when the agent needs shared local work memory across terminal commands and output, AI conversation history, recent failures, prior decisions, handoff context, recap context, or validation evidence. Search sivtr before asking the user to paste logs or repeat what happened earlier."
---

# Sivtr Memory

Sivtr is shared workspace memory for this machine.

Use it when the task needs evidence from recent terminal records, AI records, or handoff history.

Core protocol:

1. `search` the workspace.
2. `show` the exact `ref`.
3. `copy` only when you need the raw block or recent command text.

Terms:

- workspace: the current project directory used to resolve current AI sessions and terminal capture
- record: one terminal command/output block or one AI turn
- ref: a stable pointer for a record or one line inside it

Rules:

- Search first; do not ask for pasted logs until sivtr has been checked.
- Prefer `--json` for agent-to-agent or tool-to-tool use.
- Prefer `show` over copying large history.
- Use `copy --print` for raw extraction; avoid `--pick` unless the user wants interaction.
- Treat memory as evidence; verify current files or commands before claiming state.

Load only the reference you need:

- `references/commands.md` - command syntax, JSON shape, refs, and time filters
- `references/patterns.md` - common user intents mapped to retrieval steps
- `references/evidence.md` - what to trust and how to report it
