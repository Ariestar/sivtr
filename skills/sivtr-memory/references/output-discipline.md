# Output Discipline

When reporting findings from sivtr:

- Say what you searched for.
- Quote only relevant snippets.
- Distinguish terminal evidence from AI-conversation evidence.
- Do not overclaim if search results are partial.
- If no result is found, say: "I searched sivtr for X and found no matching local memory" before asking for missing context.

## Common Anti-Patterns

Avoid these:

- Asking "paste the error" before searching sivtr.
- Dumping `sivtr copy 1..100 --print` into context.
- Opening interactive pickers in automated agent runs.
- Treating prior AI dialogue as ground truth without verifying against code/tests.
- Claiming a test passed without running or retrieving actual validation output.

## Suggested Response Shapes

### Found terminal evidence

```text
I searched sivtr for "<query>" and found terminal evidence from <source/session>:

> <short relevant snippet>

This suggests <interpretation>. I will verify against <file/test> before changing code.
```

### Found prior AI decision

```text
I searched sivtr for "<query>" and found prior AI-conversation context:

> <short relevant snippet>

I will treat this as decision context, not proof of current code state.
```

### Found nothing

```text
I searched sivtr for "<query>" and found no matching local memory. I need <specific missing context>, or I can proceed by reproducing the issue locally.
```

### Final debugging answer

```text
Evidence checked:
- sivtr search: <query/result summary>
- files inspected: <paths>
- verification: <command + result>

Change/result:
- <what changed or what was found>

Remaining risks:
- <if any>
```
