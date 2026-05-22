# Workflows

## Debugging Recipe

When debugging a failure:

1. Search current memory for the likely error.

```bash
sivtr search "error|failed|panic|Traceback|Exception|exit code|FAILED" --json --limit 20
```

2. If the user mentioned a specific tool, narrow the query.

```bash
sivtr search "cargo test|could not compile|error\\[E[0-9]+\\]" --json --limit 20
```

3. Pull the last command output only if search snippets are insufficient.

```bash
sivtr copy out 1 --print
```

4. Read source files related to the error.
5. Patch surgically.
6. Run targeted verification.
7. Include the verification output in the final answer.

## Continuation Recipe

When the user says "continue", "接着", or another vague continuation:

1. Search for recent task terms from the conversation.
2. Search for explicit markers:

```bash
sivtr search "next step|TODO|blocked|decision|commit|test result|passed|failed" --json --limit 20
```

3. Summarize what the shared memory shows:
   - current goal
   - latest change/commit/test
   - open risks
   - next recommended action
4. Only ask a clarification question if multiple plausible tasks remain.

## Handoff Recipe

For a handoff to another agent:

```bash
sivtr search "current goal|next step|TODO|blocked|decision|test result|commit" --json --limit 30
sivtr copy cmd 1..10 --print
```

Then produce:

- Goal
- What changed
- Evidence from terminal/AI memory
- Tests/validation already run
- Known risks
- Next suggested steps

## Recap Recipe

For a work recap or PR summary:

1. Search for successful and failed validation:

```bash
sivtr search "test result|passed|failed|cargo test|npm test|pytest|commit" --json --limit 30
```

2. Search for decisions and measured data:

```bash
sivtr search "decision|measured|ms|speedup|before|after|risk|defer" --json --limit 30
```

3. Produce a compact timeline with evidence.
