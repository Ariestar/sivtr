# Evidence

Use this file to decide what to trust and how to report it.

## What counts

- Terminal output: strongest for commands, builds, tests, deploys, and errors
- AI dialogue: useful for intent, prior decisions, and handoff context
- Current files and tests: required when the answer depends on present reality

## Strength order

1. Current file or verified command output
2. Recent terminal evidence
3. Recent AI discussion
4. Older memory

## Rules

- Quote narrowly
- Name the source type
- Do not overclaim when search results are partial
- Do not treat AI dialogue as proof of current code state

## Output shape

Keep the answer short and explicit:

- what was searched
- what was found
- what it means
- what still needs verification

## Anti-patterns

- Asking for pasted logs before checking sivtr
- Dumping large history into context
- Calling interactive pickers in automated runs
- Claiming a test passed without running or retrieving it

## Minimal response templates

Found terminal evidence:

```text
I searched sivtr for "<query>" and found terminal evidence from <source/session>.
> <short snippet>
I will verify against <file/test> before changing code.
```

Found prior AI context:

```text
I searched sivtr for "<query>" and found prior AI-conversation context.
> <short snippet>
I will treat this as decision history, not proof of current code state.
```

Nothing found:

```text
I searched sivtr for "<query>" and found no matching local memory.
```
