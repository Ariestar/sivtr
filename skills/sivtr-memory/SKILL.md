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

Use this skill whenever local work memory may contain useful evidence. Sivtr retrieves recent terminal activity, AI conversation history, prior decisions, validation output, debugging trails, and handoff context.

Use it before asking the user to repeat context when you need to know things like:

- what just happened in the terminal or an agent session
- what command was run, what output it produced, or whether validation passed
- what the user or a previous agent decided, rejected, or planned
- where earlier debugging, build, test, lint, deploy, or research work left off
- what evidence supports a recap, handoff, status update, or next step
- whether missing or truncated current-context output exists in local memory

Do not use Sivtr as truth by itself. Treat memory as evidence to retrieve, then verify current files or commands before making claims about current state.

## Default Retrieval Workflow

Mental model to keep in mind:

- A WorkSet is `records + anchors`: `records` are the materialized backing store; `anchors` are the active positions moving through `search`, `filter`, `work`, `nav`, `zoom`, and `show`.
- Treat `@last` and `@name` as WorkSet variables. Use `--save <name>` or `sivtr var set <name> <source>` whenever a result will be reused or refined.
- Use `filter` to narrow a WorkSet, `work parts` to project record anchors into part anchors, `nav` to move anchors deterministically, and `zoom` only when you want neighboring record context.
- In shell pipelines, `@` means "read the WorkSet JSON from stdin". Do not pipe `--refs` text into `@`; either omit `--refs` in intermediate commands or use `@last` / `@name`.

1. Convert the user's vague reference into a small query.
2. Choose a source: `terminal`, `agent`, `pi`, `codex`, `claude`, `opencode`, a WorkRef selector, or a WorkSet variable such as `@last` / `@name[1,3]`.
3. Search with a small limit. Put content terms in `-m` / `--match`. Use `--last` / `--since` for time windows, `-i` / `--in` for part candidate filters, and `--kind` for part kind filters.
   - Latest terminal error: `sivtr s terminal --status fail --latest 1 --save latest_failure --refs`
   - Broader terminal error scan: `sivtr s terminal -m "Error|error|failed|fatal|not found|External command failed" --latest 20 --save error_hits --refs`
4. Save/refine WorkSet variables instead of re-running broad searches:
   - `sivtr filter @error_hits -m "more specific terms" --save narrowed --refs`
   - `sivtr filter @last[1,3] -m "narrower terms" --save focused --refs`
5. Move anchors deliberately:
   - Filter a saved set: `sivtr filter @focused --status failure --save failed_focused --refs`
   - Drill into parts: `sivtr work parts @focused --io output --kind tool_output --save output_parts --refs`
   - Deterministically move from a part to its record or neighbor: `sivtr nav @output_parts[1] '<+1>1' --refs`
   - Return to records: `sivtr work records @output_parts --save parent_records --refs`
6. Expand only useful parent records:
   - `sivtr zoom @focused[1] -C 2 --save ctx --refs`
   - `sivtr show @ctx --full`
   - `sivtr show <ref> --full`
7. Answer with evidence, then verify current files or commands when the claim depends on present repository state.
8. Ask the user only after local memory has been checked and still lacks the needed fact.

## WorkSet Flow

`search`, `filter`, `work records`, `work parts`, `nav`, and `zoom` create WorkSets. Each run saves the result to `@last`. Add `--save <name>` where supported, or use `sivtr var set <name> <source>` to keep a named WorkSet variable.

Core semantics:

- `records` are backing facts/materialized context.
- `anchors` are the active selection and the thing that moves through pipes.
- `search` searches `WorkPart`s, then outputs anchors at the current input granularity.
- `filter` applies the same WorkSet filters to any source or piped WorkSet.
- `nav` moves anchors deterministically. It does not implicitly expand children; child movement must specify `>N`.
- `show` renders at anchor granularity: record anchors show records, part anchors show just that part, line anchors show just that line.
- `zoom` maps any anchor to its parent record, then expands nearby records.

Source forms:

- `terminal`, `agent`, `pi`, `codex`, `claude`, `opencode`
- `terminal/<session>/<record>`, `<provider>/<session>/<turn>`, `<provider>/<session>/<turn>/<i|o>/<part>`, and selector variants
- `@last`, `@name`, `@name[1]`, `@name[1,3]`, `@name[1..5]`, `@name[1..3,8]`
- `@` reads a WorkSet from stdin in shell pipelines

Output behavior:

- Terminal stdout with no explicit format prints `full`.
- Piped stdout with no explicit format prints WorkSet JSON for the next command.
- `--json` is `--format workset`.
- `--refs` is `--format refs`.
- `--full` on `show` is `--format full`.

Pipeline example (pipe WorkSet JSON; do not add `--refs` in intermediate steps):

```bash
sivtr s agent -m "panic|failed" --latest 20 --save failures \
  | sivtr filter @ -m "cargo|test" --save test_failures \
  | sivtr zoom @ -C 1 --save failure_ctx \
  | sivtr show @ -f timeline
```

Anchor movement example:

```bash
sivtr s pi -m "git push|main -> main" --latest 10 --save push_hits --refs
sivtr work parts @push_hits --io output --kind tool_output --save push_outputs --refs
sivtr filter @push_outputs -m "main -> main" --save exact_output --refs
sivtr show @exact_output --full
```

Anchor navigation example:

```bash
sivtr s pi -m "panic|failed" --latest 10 --save hits --refs
sivtr work parts @hits --io output --save output_hits --refs
sivtr nav @output_hits[1] '<' --refs        # part/line -> parent record
sivtr nav @output_hits[1] '<+1>1' --refs    # parent record -> next record -> first child
sivtr nav @output_hits[1] '<[-2..+2]' --refs # parent record sibling window
sivtr nav @output_hits[1] '~' --refs        # containing session records
```

Named WorkSet variable example:

```bash
sivtr s agent -m "decision|TODO|next step" --latest 20 --save hits --refs
sivtr filter @hits -m "workset|zoom|show" --save narrowed --refs
sivtr var list
sivtr zoom @narrowed[1] -C 2 --save ctx --refs
sivtr show @ctx --full
```

## Non-Interactive Safety Rules

- Prefer non-interactive commands: `sivtr s ... --refs`, `sivtr s ... -f <timeline|compact|md>`, `sivtr show ... --full`, `sivtr show ... --json`.
- Use WorkSet variables (`@last`, `@name`) and `@` pipelines for chaining; save reusable intermediate sets with `--save <name>` or `sivtr var set <name> <source>`.
- Do not open TUI pickers (`--pick`, hotkey picker) unless the user explicitly wants interactive selection.
- Do not run `sivtr clear`, hotkey start/stop, shell init, or config mutation unless the user explicitly asks.
- Avoid clipboard-oriented workflows in agent retrieval. Use refs from `search` and expand them with `show` / `zoom`.
- Avoid dumping huge histories into the model. Search narrowly first, then expand only the relevant records.
- If `sivtr` is not installed or no session log exists, say so briefly and continue with normal tools. Do not invent memory results.

## Load References as Needed

References are relative to this skill directory.

- `references/commands.md` — command syntax, WorkSet handling, and token budget.
- `references/patterns.md` — common user intents mapped to retrieval steps.
- `references/evidence.md` — what counts as evidence and how to report it.

Read only the file needed for the current task.
