---
title: Remote Collaboration Memory
description: Scenario playbook for reading a teammate's terminal or agent memory through a named remote.
---

## The scenario

You are working on a bug that another teammate's agent already investigated. With permission, you want to search their agent memory directly and see what was tried — without leaving your machine or pasting logs.

Feature setup, safety defaults, and command map live in [Remote Access](/usage/remote-access/). This page is the scenario path.

## What you would say

```text
What did Alice's agent already try on this bug?
Show me Bob's validation output before I continue.
```

## Setup once

Both devices need `sivtr` installed. The daemon auto-starts when share/remote commands need it.

On the device that owns the workspace:

```bash
sivtr share                   # pick workspace (Enter = current); create share only
sivtr share invite <name>     # single-use invite (stdout = bare key)
```

On the other device, from the workspace where you want the remote:

```bash
sivtr remote add desk <invite>
sivtr remote test desk
```

## Daily workflow

```bash
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr s desk:agent -m "panic|failed|decision" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr zoom desk:agent/<session>/3 -C 2 --save remote_ctx --refs
sivtr show @remote_ctx -f timeline
sivtr filter @remote_hits -m "cargo test" --save remote_tests --refs
```

## Keep access tidy

Owner:

```bash
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
```

Peer:

```bash
sivtr remote list
sivtr remote remove desk
```

## Demo video outline

Show two machines: owner runs `sivtr share` then `share invite`, consumer redeems the invite as remote `desk`, searches `desk:terminal` for a failure, zooms the hit, and continues the fix from the teammate's evidence.
