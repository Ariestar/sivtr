---
title: Search and Show Results
description: Search current workspace memory and print exact refs.
---

`sivtr search` queries captured terminal records and supported AI workspace sessions. `sivtr show` prints the content behind an exact ref.

Use them together when an interactive picker is too much and you want scriptable memory for a human workflow, an agent prompt, or another tool. They are also the safest primitives for skills because they can run non-interactively and return exact refs.

For example, a "fix the terminal error" skill can start with:

```bash
sivtr search terminal --status failure --latest 1 --format json
```

and a "recent work timeline" skill can use a timeline renderer:

```bash
sivtr search agent --since today --sort oldest --format timeline
sivtr search terminal --since today --sort oldest --format timeline
```

## Search targets

Search is target-first:

```bash
sivtr search terminal
sivtr search agent
sivtr search codex
sivtr search claude
sivtr search opencode
sivtr search pi
```

Targets can narrow to a session, record/turn, and line:

```bash
sivtr search pi/019e5941 --match "cargo test"
sivtr search terminal/session_13104/3/12 --format json
sivtr search pi/019e5941/3-5,7 --match "cargo test"
sivtr search pi/019e5941/3/5-7,10 --format json
```

Record/turn and line segments are 1-based and accept `3`, `3-5`, `3,7`, or `3-5,7`. Use `*` as a wildcard segment. Search selectors narrow the input scope; search output still returns concrete refs.

Use `agent` for every supported AI provider, or a provider name for one provider.

## Content filters

```bash
sivtr search terminal --match "panic|failed"
sivtr search agent --match "TODO|next step|decision"
sivtr search pi --match "workspace picker" --in title
```

`--match` is a case-insensitive regex. `--in` chooses the field:

| Field | Searches |
| --- | --- |
| `content` | Combined record content. This is the default. |
| `title` | Record/dialogue title |
| `session` | Session id/title |
| `input` | User input / command input |
| `output` | Assistant output / command output |
| `command` | Terminal command text |
| `all` | All searchable text |

## Time filters

```bash
sivtr search agent --since today --format timeline
sivtr search terminal --since yesterday --until today --format md
sivtr search pi --last 2h --format compact
```

Time filters accept RFC3339 timestamps, Unix seconds/milliseconds, relative durations like `30m`, `2h`, `7d`, and aliases such as `today`, `yesterday`, `tomorrow`, `this morning`, `this afternoon`, `this evening`, `tonight`, and `now`.

## Status, duration, and sorting

```bash
sivtr search terminal --status failure --latest 1 --format json
sivtr search terminal --exit-code 101 --format timeline
sivtr search terminal --min-duration 500ms --sort duration --format compact
```

Useful sorts:

- `newest`
- `oldest`
- `duration`
- `duration-asc`
- `exit-code`
- `exit-code-asc`

`--latest <N>` first keeps the latest N matching records. `--sort` then controls final presentation order.

## Output formats

```bash
sivtr search agent --since today --format timeline
sivtr search agent --since today --format compact
sivtr search agent --since today --format md
sivtr search agent --since today --format json
```

Formats are views over the same search result set, not separate APIs for humans vs agents. Pick the format that best fits the next step:

| Format | Good for |
| --- | --- |
| `timeline` | Chronological scanning, handoff reconstruction, spotting gaps. Easy for both humans and agents to read. |
| `compact` | Short time/source/title lists when you want low-noise context. |
| `md` | Markdown bullets for notes, reports, prompts, or handoff drafts. |
| `json` | Structured refs and snippets when another program will parse the output. |

The default is `json` so scripts get a stable shape when no format is specified. Agents can also read `timeline`, `compact`, or `md` when the task is interpretive rather than programmatic.

## Show a ref

Refs/selectors have this shape:

```text
source/session[/record-or-turn[/line]]
source/session/record/<i|o>/<part>
```

A concrete ref points at one record, one line, or one part. As command input, the record/turn and line segments can also be selectors such as `3-5,7`; output refs remain concrete anchors. Part refs use `i` (input) or `o` (output) followed by a 1-based part index.

Print a record or turn:

```bash
sivtr show pi/<session>/<turn>
sivtr show terminal/<session>/<record>
```

Print one 1-based line:

```bash
sivtr show claude/<session>/<turn>/<line>
sivtr show terminal/<session>/<record>/<line>
```

Print a specific input or output part:

```bash
sivtr show codex/<session>/<turn>/o/1
sivtr show terminal/<session>/<record>/i/2
```

Print multiple records or lines with selector syntax:

```bash
sivtr show pi/<session>/3-5,7
sivtr show pi/<session>/3/5-7,10
```

Use JSON for machine-readable output:

```bash
sivtr show pi/<session>/<turn>/<line> --json
```

## Practical loop

1. Search narrowly enough to get evidence:

   ```bash
   sivtr search terminal --status failure --latest 1 --format json
   sivtr search agent --match "current task|failed|TODO" --since today --format timeline
   ```

2. Pick the result ref you care about.
3. Print the surrounding record:

   ```bash
   sivtr show <source/session/record-or-turn>
   ```

4. Use exact line refs when you need compact citations, script input, or context handles for another agent.
