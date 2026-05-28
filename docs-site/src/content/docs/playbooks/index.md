---
title: Playbooks
description: Community-style workflows that combine sivtr memory, skills, and agents.
---

Playbooks show how `sivtr` works in practice: real scenarios where an agent uses local workspace memory.

## Demos

These short recordings show the core loop: capture local work, search it, narrow the evidence, and hand exact context to the next command or agent.

<div class="demo-grid">
  <figure>
    <img src="/demo/1.gif" alt="Search recent terminal output with sivtr" />
    <figcaption>Find the latest terminal evidence.</figcaption>
  </figure>
  <figure>
    <img src="/demo/2.gif" alt="Browse and reuse captured workspace memory" />
    <figcaption>Browse and reuse captured context.</figcaption>
  </figure>
  <figure>
    <img src="/demo/3.gif" alt="Build a timeline from local agent and terminal memory" />
    <figcaption>Turn recent work into a timeline.</figcaption>
  </figure>
  <figure>
    <img src="/demo/4.gif" alt="Pass a named memory variable through a command chain" />
    <figcaption>Save matches as variables and keep working.</figcaption>
  </figure>
</div>

## Playbook index

| Playbook | What it demonstrates |
| --- | --- |
| [Fix the latest terminal error](/playbooks/fix-terminal-error/) | The agent finds the failure, patches the issue, and verifies the fix. |
| [Build a recent work timeline](/playbooks/recent-work-timeline/) | The agent reconstructs what you worked on from timestamps, commands, and agent dialogues. |
| [Continue after interruption](/playbooks/continue-after-interruption/) | The agent searches memory to find the most recent work thread. |
| [Agent handoff](/playbooks/agent-handoff/) | The agent prepares a structured handoff with evidence and next steps. |
| [Remote collaboration memory](/playbooks/remote-collaboration-memory/) | Future direction: search a teammate's agent memory with permission. |
