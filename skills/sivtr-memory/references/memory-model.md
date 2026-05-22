# Memory Model

Treat `sivtr` as a local event memory, not merely a clipboard helper.

It can surface evidence from:

- terminal commands
- terminal stdout/stderr
- test/build errors
- AI dialogues
- prior decisions and plans
- validation evidence
- recent project activity

The agent should not depend on the human to shuttle this context manually.

## Product Insight

Sivtr enables a different human-agent interaction pattern: the human and agent share one local work memory.

The human can say:

> fix the thing that just failed

The agent can then search the terminal + AI trail to find the actual failure, prior attempts, and validation evidence.

## What Counts as Evidence

Terminal evidence is stronger for operational claims:

- command output
- test results
- build logs
- deploy output
- actual error text

AI-conversation evidence is useful for intent and decision history:

- why a direction was chosen
- what was deferred
- what assumptions were made
- what the previous agent planned

Do not treat prior AI dialogue as ground truth for code state. Verify against files and tests when the answer depends on current reality.

## Boundaries

This skill only teaches the agent how to use the existing `sivtr` CLI. It does not imply unavailable commands exist.

Do not invent future commands such as `sivtr show`, `sivtr recap`, `sivtr handoff`, `sivtr why`, or `sivtr doctor` unless the installed CLI actually supports them.
