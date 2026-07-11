---
title: Search and Show Results
description: Search current workspace memory and print exact refs.
---

`sivtr search` queries captured terminal records and supported AI workspace sessions. `sivtr filter` narrows existing WorkSets. `sivtr nav` moves anchors through parent/child/sibling/session structure. `sivtr show` prints the content behind refs or WorkSets.

Use them together when an interactive picker is too much and you want scriptable memory for a human workflow, an agent prompt, or another tool. They are also the safest primitives for skills because they can run non-interactively and return exact refs or WorkSet JSON.

For example, a "fix the terminal error" skill can start with:

```bash
sivtr search terminal --status failure --latest 1 --json
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
sivtr search hermes
sivtr search opencode
sivtr search pi
```

Targets can narrow to a session, record/turn, and line:

```bash
sivtr search pi/019e5941 --match "cargo test"
sivtr search terminal/session_13104/3/12 --format workset
sivtr search pi/019e5941/3-5,7 --match "cargo test"
sivtr search pi/019e5941/3/5-7,10 --format workset
```

Record/turn and line segments are 1-based and accept `3`, `3-5`, `3,7`, or `3-5,7`. Use `*` as a wildcard segment. Search selectors narrow the input scope; search output still returns concrete refs.

Use `agent` for every supported AI provider, or a provider name for one provider.

Targets can also use an origin prefix (`origin:body`) for another local workspace name or a mounted remote alias:

```bash
sivtr search desk:terminal --status failure --latest 5 --refs
sivtr search desk:agent -m "decision|failed" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/o/1 --full
sivtr show docs:codex/4
```

Origins come from `sivtr remote add <alias> ...` or `sivtr wb list`. Feature guide: [Remote Access](/usage/remote-access/).

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
sivtr search terminal --status failure --latest 1 --json
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
sivtr search agent --since today --format workset
```

Formats are views over the same search result set, not separate APIs for humans vs agents. Pick the format that best fits the next step:

| Format | Good for |
| --- | --- |
| `timeline` | Chronological scanning, handoff reconstruction, spotting gaps. Easy for both humans and agents to read. |
| `compact` | Short time/source/title lists when you want low-noise context. |
| `md` | Markdown bullets for notes, reports, prompts, or handoff drafts. |
| `workset` | Structured refs and materialized records when another command or program will parse the output. |
| `refs` | Plain refs, one per line, for quick inspection or copy/paste. |

Terminal stdout defaults to `full`; piped stdout defaults to `workset`. Use `--json` as a convenient alias for `--format workset`. Agents can also read `timeline`, `compact`, or `md` when the task is interpretive rather than programmatic.

## Filter WorkSets

Use `filter` when you already have a WorkSet and want to narrow it without re-running a broad search:

```bash
sivtr search terminal --status failure --latest 20 --save failures --refs
sivtr filter @failures --match "panic|compile" --save focused --refs
sivtr filter @focused --parts --io output --kind tool_output --refs
```

In a shell pipeline, `@` reads WorkSet JSON from stdin:

```bash
sivtr search terminal --json | sivtr filter @ -m error --refs
```

Do not pipe `--refs` output into `@`; `@` expects WorkSet JSON.

## Navigate anchors

Use `nav` when the movement path matters. Motion is deterministic and does not expand children by default.

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
sivtr nav @focused[1] '<' --refs
sivtr nav @focused[1] '<+1>1' --refs
sivtr nav @focused[1] '<[-2..+2]' --refs
sivtr nav @focused[1] '~' --refs
```

Use `zoom` for simple neighboring record context around hits.

## WorkSet variables

Use `var` when a WorkSet should survive as named local memory:

```bash
sivtr var set ctx @last
sivtr var list
sivtr var merge ctx @focused @last[1]
sivtr var drop ctx @noise
sivtr show @ctx --full
```

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

Use WorkSet output for machine-readable piping:

```bash
sivtr show @ctx --json
```

## Practical loop

1. Search narrowly enough to get evidence:

   ```bash
   sivtr search terminal --status failure --latest 1 --refs
   sivtr search agent --match "current task|failed|TODO" --since today --format timeline
   ```

2. Save and narrow reusable result sets:

   ```bash
   sivtr search agent --match "decision|TODO" --latest 20 --save hits --refs
   sivtr filter @hits --match "workspace|nav|filter" --save focused --refs
   ```

3. Move or expand anchors when needed:

   ```bash
   sivtr nav @focused[1] '<[-1..+1]' --refs
   sivtr zoom @focused[1] -C 2 --save ctx --refs
   ```

4. Print exact content:

   ```bash
   sivtr show @ctx --full
   sivtr show <source/session/record-or-turn>
   ```

5. Use exact part/line refs when you need compact citations, script input, or context handles for another agent.
