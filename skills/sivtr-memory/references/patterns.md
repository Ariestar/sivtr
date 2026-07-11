# Patterns

Use this file when you need to turn a vague user request into a retrieval plan. Command syntax itself lives in `references/commands.md`.

## Recent Failure

When the user says the last command failed, search terminal evidence first.

- Start with `sivtr s terminal --status fail --latest 1 --refs`.
- If status is sparse, search terminal error text with `sivtr s terminal -m "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --latest 20 --refs`.
- Narrow by tool, language, workspace, or time window when obvious.
- Expand the strongest result with `sivtr zoom @last[1] -C 2 --save failure_ctx --refs`.
- Read exact content with `sivtr show @failure_ctx --full`.
- Inspect related files and verify locally before claiming current state.

## Continue Work

When the user says "continue", reconstruct the active thread before guessing.

- Extract structured constraints from the user's wording: provider/source, cwd/workspace, time window, content topic, and whether they mean content, dialogue title, or session title.
- Express structured constraints with source selectors and filters: `terminal`, `agent`, provider sources, `@last`, `@name`, WorkRef selectors, `--cwd`, `--last`, `--since`, `--until`, `-i` / `--in`.
- Start with a broad topic search and save the WorkSet.
- Search for `next step`, `TODO`, `blocked`, `decision`, `commit`, `test result`, `passed`, and `failed`.
- Refine with `sivtr filter @<name> -m "<topic>" --save <narrowed> --refs`.
- Expand the best records with `zoom`.
- If one thread is obvious, summarize it and keep going.
- If more than one thread is plausible, ask the user which one to continue.

Example:

```bash
sivtr s agent -m "next step|TODO|blocked|decision|commit|test result|passed|failed" --latest 30 --save recent --refs
sivtr filter @recent -m "<topic>" --save thread --refs
sivtr zoom @thread[1] -C 2 --save thread_ctx --refs
sivtr show @thread_ctx --full
```

## Prior Decision

When the user asks why something was chosen earlier:

- Search for the decision terms and related discussion.
- Save the first-pass WorkSet.
- Refine the WorkSet with exact topic terms.
- Expand the relevant prior dialogue with `zoom` when the match is too small.
- Treat the result as intent history.
- Verify current files or tests before making a claim about current state.

## Handoff

When another agent needs to continue the work:

- Search for goal, next step, decisions, and validation evidence.
- Save and refine WorkSets until the thread is small.
- Expand the few strongest matches.
- Report goal, current state, evidence, tests, risks, and next step.

Example:

```bash
sivtr s agent -m "goal|next step|decision|validation|cargo test|clippy|passed|failed" --latest 50 --save handoff_hits --refs
sivtr filter @handoff_hits -m "<project-or-feature>" --save handoff --refs
sivtr zoom @handoff[1..3] -C 1 --save handoff_ctx --refs
sivtr show @handoff_ctx -f timeline
```

## Recap

When the user wants a summary of what happened:

- Search for successful and failed validation.
- Search for decisions and measurable changes.
- Use WorkSet selectors to keep the evidence small.
- Expand only refs that anchor the timeline.
- Produce a compact timeline with evidence, not a transcript dump.

## Remote Teammate Memory

When the user asks what another machine, teammate, or mounted alias already tried:

- Check mounts and local workspace labels first: `sivtr remote list`, `sivtr wb list`.
- Search the origin-prefixed source, not local defaults: `sivtr s desk:agent -m "<topic>" --latest 20 --save remote_hits --refs`.
- Expand only the strongest hits with `zoom` / `show`.
- Treat remote results as evidence from another device; verify current local files before acting.
- Do not run `sivtr share`, `remote add`, or `share revoke` unless the user explicitly asks.

Example:

```bash
sivtr remote list
sivtr s desk:agent -m "panic|failed|decision|next step" --latest 20 --save remote_hits --refs
sivtr filter @remote_hits -m "<bug-topic>" --save remote_thread --refs
sivtr zoom @remote_thread[1] -C 2 --save remote_ctx --refs
sivtr show @remote_ctx --full
```

## Missing Memory

When no useful evidence is found:

- Say that sivtr did not find matching local memory.
- State the specific missing fact.
- Reproduce the issue locally or ask the user for the missing source.

## Permission Required

When the task involves deletion, reset, or config mutation:

- Search for context if helpful.
- Stop before the destructive step.
- Ask for explicit confirmation with the exact target.
