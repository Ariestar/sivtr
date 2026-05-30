---
title: Selectors and Filters
description: Shared syntax for recent-item selectors, sessions, regex filters, line filters, and refs.
---

Several `sivtr` commands share the same small syntax for choosing and trimming text. This page collects those rules in one place.

## Recency selectors

Selectors choose recent command blocks or AI units.

| Selector | Meaning |
| --- | --- |
| omitted | Latest matching item |
| `1` | Latest matching item |
| `2` | Second latest matching item |
| `2..4` | Range of recent matching items |

Examples:

```bash
sivtr copy out
sivtr copy out 2
sivtr copy in 2..4
sivtr copy claude out 2
sivtr copy codex 2..4
```

Selectors are newest-first because most reuse targets are recent.

## Diff selectors

`diff` uses the same recency numbering, but each side must resolve to exactly one command block:

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
```

Range selectors such as `2..4` are not valid for one side of a diff.

## AI session selection

Agent provider commands can select a session separately from the item selector:

```bash
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy claude out --session 3
```

`--session N` picks the Nth newest selectable session from the same ordering shown in picker flows. `--session ID` matches a session id or id prefix.

## Regex filters

`--regex <PATTERN>` keeps only matching lines after selected text is assembled:

```bash
sivtr copy out --regex panic
sivtr copy claude tool --regex "error|failed"
```

Use quotes when the shell would otherwise interpret characters in the pattern.

## Line filters

`--lines <SPEC>` keeps 1-based line ranges after selected text is assembled:

```bash
sivtr copy out --lines 10:20
sivtr copy out --lines 1,3,8:12
sivtr copy codex all --lines 1:40
```

Common forms:

| Spec | Meaning |
| --- | --- |
| `5` | Line 5 |
| `1:5` | Lines 1 through 5 |
| `10:20` | Lines 10 through 20 |
| `1,3,8:12` | Lines 1, 3, and 8 through 12 |

When both `--regex` and `--lines` are set, `--regex` runs first and `--lines` runs on the filtered result.

## WorkSet references

WorkSet commands (`search`, `filter`, `nav`, `zoom`, `show`, `work records`, and `work parts`) share source forms:

| Source | Meaning |
| --- | --- |
| `@last` | Most recent WorkSet produced by a WorkSet command. |
| `@name` | Named WorkSet saved by `--save name` or `sivtr var set name`. |
| `@name[1,3..5]` | 1-based slice of a saved WorkSet. Discrete selectors keep the requested order. |
| `@` | Read WorkSet JSON from stdin. Do not pipe `--refs` text into `@`. |

WorkSets contain materialized `records` plus active `anchors`. `filter` narrows anchors, `nav` moves anchors, `var` remembers anchors, and `show` renders anchors.

## Anchor motion

`sivtr nav <source> <motion>` uses a small deterministic motion syntax. It does not default-expand children.

| Motion | Meaning |
| --- | --- |
| `<` | Parent. Part/line to record; record to containing session records. |
| `>N` | Nth child, 1-based. Record children are its parts. |
| `+N` | Next sibling by N at the current level. |
| `-N` | Previous sibling by N at the current level. |
| `[A..B]` | Sibling window at the current level. |
| `~` | Containing session records. |

Examples:

```bash
sivtr nav @hit '<' --refs
sivtr nav @hit '<+1>1' --refs
sivtr nav @hit '<[-2..+2]' --refs
sivtr nav @hit '~' --refs
```

## Prompt rewriting

Input-capable command-block modes can rewrite the copied prompt:

```bash
sivtr copy in --prompt ":"
sivtr copy --prompt ">"
```

If the prompt does not end with whitespace, `sivtr` inserts one space before the command.

## ANSI preservation

Use `--ansi` to copy ANSI-decorated text when the source has preserved ANSI content:

```bash
sivtr copy out --ansi
```

Plain text remains the default because it is stable for search, issue reports, and AI prompts.

## Workspace refs

`search --format json` emits refs that `show` can print:

```text
source/session[/dialogue[/line]]
```

Examples:

```bash
sivtr show claude/<session>
sivtr show claude/<session>/<dialogue>
sivtr show claude/<session>/<dialogue>/<line>
sivtr show terminal/current/<block>
sivtr show terminal/current/<block>/<line>
```

Dialogue and line indices are 1-based.
