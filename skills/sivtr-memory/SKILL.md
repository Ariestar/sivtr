---
name: sivtr-memory
description: "Retrieve shared local work memory: terminal activity, AI conversation history, prior decisions, validation evidence, debugging trails, recaps, and handoff context. Use before asking the user to repeat local work context."
---

# Sivtr Memory

Sivtr is the shared local work memory for this machine.

Use it before asking the user to paste logs, repeat decisions, or restate earlier work.

Core rule:

> Search for evidence first. Expand only the smallest relevant context. Ask when memory is missing, ambiguous, stale, or permission is required.

## When to Use

Use this skill whenever local work memory may contain useful evidence. Sivtr is the shared retrieval layer for recent terminal
activity, AI conversation history, prior decisions, validation output, and
handoff context.

Use it before asking the user to repeat context when you need to know things
like:

- what just happened in the terminal or an agent session
- what command was run, what output it produced, or whether validation passed
- what the user or a previous agent decided, rejected, or planned
- where earlier debugging, build, test, lint, deploy, or research work left off
- what evidence supports a recap, handoff, status update, or next step
- whether missing or truncated current-context output exists in local memory

Do not use Sivtr as truth by itself. Treat memory as evidence to retrieve, then
verify current files or commands before making claims about current state.

## Default Retrieval Workflow

1. Convert the user's vague reference into a query.
2. Search with a small limit and a format that fits the task. Put the source/layer in the
   target selector (`terminal`, `agent`, `pi`, `codex`, `claude`, or
   `opencode`) and put content terms in `--match`. Use `--last`/`--since` for
   time windows and `--in` for field constraints. Use `--format json` when you
   need structured fields; use `timeline`, `compact`, or `md` when that is
   easier to reason over.
   - For "latest terminal error" / "最新终端报错", start with:
     `sivtr search terminal --status failure --format json --latest 1`.
   - If status is unknown or insufficient, broaden with:
     `sivtr search terminal --match "Error|error|failed|fatal|not found|External command failed" --format json --latest 20`.
3. Inspect result source/session/dialogue/snippet.
4. If the snippet is enough, answer with evidence.
5. If more context is needed, expand the returned ref with `sivtr show ... --json` or run a narrower second search.
6. Ask the user only after local memory has been checked and still lacks the needed fact.

## Non-Interactive Safety Rules

- Prefer non-interactive commands: `sivtr search ... --format <timeline|compact|md|json>`, `sivtr show ... --json`.
- Do not open TUI pickers (`--pick`, hotkey picker) unless the user explicitly wants interactive selection.
- Do not run `sivtr clear`, hotkey start/stop, shell init, or config mutation unless the user explicitly asks.
- Avoid clipboard-oriented workflows in agent retrieval. Use refs from `search` and expand them with `show`.
- Avoid dumping huge histories into the model. Search narrowly first, then expand only the relevant block/dialogue.
- If `sivtr` is not installed or no session log exists, say so briefly and continue with normal tools. Do not invent memory results.

## Load References as Needed

References are relative to this skill directory.

- `references/commands.md` — command syntax, JSON handling, and token budget.
- `references/patterns.md` — common user intents mapped to retrieval steps.
- `references/evidence.md` — what counts as evidence and how to report it.

Read only the file needed for the current task.
