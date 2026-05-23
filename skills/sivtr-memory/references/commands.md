# Command Cookbook

Use these commands as starting points. This file is the single source for `sivtr` command syntax.
Prefer small, targeted queries over dumping large histories.

## Search Commands

### General search

```bash
sivtr search "<case-insensitive-regex>" --json --limit 20
```

### Search terminal + AI memory for common errors

```bash
sivtr search "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --json --limit 20
```

### Rust failures

```bash
sivtr search "error\\[E[0-9]+\\]|panicked|test result: FAILED|could not compile|borrow|lifetime" --json --limit 20
```

### JavaScript / TypeScript failures

```bash
sivtr search "TypeError|ReferenceError|TS[0-9]+|npm ERR|pnpm|vite|webpack|ELIFECYCLE|failed" --json --limit 20
```

### Python failures

```bash
sivtr search "Traceback|ModuleNotFoundError|ImportError|AssertionError|pytest|FAILED|Exception" --json --limit 20
```

### Previous decisions or AI discussion

```bash
sivtr search "lazy load|workspace TUI|metadata scan|decision|TODO|next step" --json --limit 20
```

### Search titles instead of content

```bash
sivtr search "workspace picker" --scope session --json --limit 20
sivtr search "cargo test" --scope dialogue --json --limit 20
```

### Provider-specific search

```bash
sivtr search "<query>" --provider codex --json --limit 20
sivtr search "<query>" --provider claude --json --limit 20
```

## JSON Handling

Treat `--json` output as structured evidence, not as a free-form transcript.

`sivtr search --json` returns a wrapper with `query`, `scope`, `cwd`,
`match_count`, and `results`. Inspect these result fields first:

- `ref`: stable reference for follow-up expansion
- `kind`: `shell` or `ai`
- `timestamp`: how recent it is
- `title.session`: session title
- `title.dialogue`: dialogue or command block title, when available
- `content`: matched line or extracted content

Expected result item shape:

```json
{
  "ref": "terminal/current/12/8",
  "kind": "shell",
  "timestamp": "...",
  "title": {
    "session": "current shell",
    "dialogue": "cargo test"
  },
  "content": "test result: FAILED"
}
```

Use `ref` for precise follow-up. Do not infer provider/session identity from
display text when a `ref` is available.

## Expansion Commands

Use expansion after search identifies a target. Prefer small, precise expansions.

### Show a matched ref

Use `show` when search returned a `ref` and you need exact content.

```bash
sivtr show "<ref>" --json
sivtr show "terminal/current/12/8" --json
```

Refs have this shape:

```text
source/session[/dialogue[/line]]
```

### Last command output

```bash
sivtr copy out 1 --print
```

### Last command input + output

```bash
sivtr copy 1 --print
```

### Recent command list only

```bash
sivtr copy cmd 1..10 --print
```

### A small recent range

```bash
sivtr copy 1..3 --print
```

Do not copy large ranges unless the task explicitly requires a full transcript.

## Query Construction Tips

- Use the exact tool name when known: `cargo test`, `pytest`, `npm ERR`, `wrangler deploy`.
- Include high-signal error tokens: `panic`, `Traceback`, `TS2307`, `error[E`, `exit code`.
- Search for decision words when reconstructing context: `decision`, `defer`, `blocked`, `next step`, `TODO`.
- Start with `--limit 20`; increase only if the result set is clearly incomplete.

## Token Budget

- Start with `--limit 20` for normal searches.
- Use `--limit 30` only for handoff or recap work.
- Narrow the query before increasing the limit.
- Prefer `sivtr show "<ref>" --json` when search returns a useful ref.
- Prefer `sivtr copy out 1 --print` for the latest output.
- Prefer `sivtr copy 1..3 --print` for a small range.
- Avoid ranges larger than `1..10` unless the task needs a transcript.
