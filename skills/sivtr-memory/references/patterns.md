# Patterns

Use this file when a user intent needs a retrieval plan. Command syntax lives in `references/commands.md`.

## Recent failure

Search error terms first.

1. Run the error search from `commands.md`.
2. Narrow by tool or language if the project is obvious.
3. `show` the strongest ref.
4. Verify the related file or test before changing code.

## Continue work

Reconstruct the active thread before guessing.

1. Search for `next step`, `TODO`, `blocked`, `decision`, `commit`, `test result`, `passed`, and `failed`.
2. Expand only the most relevant ref.
3. Summarize the active state, then continue.
4. Ask which thread to continue only if more than one is plausible.

## Prior decision

Treat memory as intent history, not current code truth.

1. Search for decision terms and related discussion.
2. Expand the exact prior ref when the snippet is too small.
3. Verify code or tests before making a claim about current state.

## Handoff

When another agent needs to continue:

1. Search for goal, next step, decisions, and validation evidence.
2. Expand the few strongest refs.
3. Report goal, current state, evidence, tests, risks, and next step.

## Recap

When the user wants a summary:

1. Search for successful and failed validation.
2. Search for decisions and measurable changes.
3. Expand only the refs that anchor the timeline.
4. Return a compact timeline, not a transcript dump.

## Missing memory

When nothing useful is found:

1. Say sivtr did not find matching local memory.
2. State the specific missing fact.
3. Reproduce the issue locally or ask for the missing source.

## Copy only when needed

Use `copy` only when the raw block is the useful unit.

- command text for prompts or another tool
- a small terminal range for a handoff
- a provider transcript when `show` is not enough

Keep copied ranges small.
