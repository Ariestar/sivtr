# Patterns

Use this file when you need to turn a vague user request into a retrieval plan.
Command syntax itself lives in `references/commands.md`.

## Recent failure

When the user says the last command failed, search for the likely error first.

- Start with the error search from `commands.md`
- Narrow by tool or language if the project is obvious
- If search returns a useful `ref`, expand it with `sivtr show "<ref>" --json`
- If there is no useful ref and the latest terminal output matters, expand only the latest output
- Then inspect the related files and verify locally

## Continue work

When the user says "continue", reconstruct the active thread before guessing.

- Search for `next step`, `TODO`, `blocked`, `decision`, `commit`, `test result`, `passed`, and `failed`
- Use returned refs to expand only the most relevant dialogue or line
- If one thread is obvious, summarize it and keep going
- If more than one thread is plausible, ask the user which one to continue

## Prior decision

When the user asks why something was chosen earlier:

- Search for the decision terms and related discussion
- Use refs to expand the relevant prior dialogue when the matched line is too small
- Treat the result as intent history, not current code truth
- Verify the code or tests before making a claim about current state

## Handoff

When another agent needs to continue the work:

- Search for goal, next step, decisions, and validation evidence
- Expand refs for the few strongest matches
- Pull only a small command range when refs do not capture the needed terminal context
- Report goal, current state, evidence, tests, risks, and next step

## Recap

When the user wants a summary of what happened:

- Search for successful and failed validation
- Search for decisions and measurable changes
- Expand only refs that anchor the timeline
- Produce a compact timeline with evidence, not a transcript dump

## Missing memory

When no useful evidence is found:

- Say that sivtr did not find matching local memory
- State the specific missing fact
- Either reproduce the issue locally or ask the user for the missing source

## Permission required

When the task involves deletion, reset, or config mutation:

- Search for context if helpful
- Stop before the destructive step
- Ask for explicit confirmation with the exact target
