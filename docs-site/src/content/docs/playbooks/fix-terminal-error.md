---
title: Fix the Latest Terminal Error
description: Let the agent find and fix the latest terminal failure.
---

## The scenario

You run a command, it fails, and you want the agent to fix it. With `sivtr`, the agent can search the latest terminal output, inspect the relevant files, patch the issue, and verify the result.

## What you say

```text
Fix the terminal error.
```

Run this after a command fails:

## How it works

The agent searches recent workspace memory for failure signals:

```bash
sivtr search terminal --match "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --format json --limit 20
sivtr search terminal --status failure --latest 1 --format json
```

If the search snippet is too short, it expands the latest command output:

```bash
sivtr copy out 1 --print
```

Then it reads the relevant source files, patches the issue, reruns the failing command, and reports the result with verification output.

## Example interaction

```text
User: Fix the terminal error.

Agent: I found the latest failure in sivtr memory: `cargo test` failed
with error[E0432] — unresolved import `crate::config::legacy`.
I removed the stale import in src/commands/show.rs, reran `cargo test`,
and all 42 tests now pass.
```

## Demo video outline

1. Run a command that fails.
2. Tell the agent: "Fix the terminal error."
3. Show the agent searching `sivtr` for the failure.
4. Show the patch and verification output.
