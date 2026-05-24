# Command Cookbook

Use these commands as starting points. This file is the single source for `sivtr` command syntax.
Prefer small, targeted queries over dumping large histories.

## Search Commands

### Search option reference

`sivtr search` uses a target selector plus filters. The target decides where to
search; filters decide which records match. Content search is a filter via
`--match`.

```bash
sivtr search <target> \
  --match <case-insensitive-regex> \
  --in <content|title|session|input|output|command|all> \
  --status <success|failure|unknown> \
  --cwd <path> \
  --last <duration> \
  --since <time> \
  --until <time> \
  --latest <n> \
  --limit <n> \
  --json
```

Targets:

- `terminal`: terminal command records.
- `agent`: AI/agent conversation records from all providers.
- `codex`, `claude`, `pi`, `opencode`: one provider's conversation records.
- `terminal/<session>/<record>/<line>` or `<provider>/<session>/<turn>/<line>`:
  ref-like narrowing. Trailing segments are optional. `*` means wildcard.

Filters:

- `--match`: case-insensitive regex content filter.
- `--in`: field for `--match`; default is `content`.
- `--status`: filter by command/record outcome.
- `--cwd`: choose the workspace used to resolve current AI sessions. Omit it
  when already running in the target repo.
- `--last`: relative time window (`30m`, `2h`, `7d`).
- `--since` / `--until`: bound search by RFC3339 time, Unix seconds/millis, or
  relative durations.
- `--latest`: return the latest N matching records.
- `--limit`: cap printed results; use when you want a display cap different
  from `--latest`.
- `--json`: required for agent use unless the user explicitly wants raw text.

### General search

```bash
sivtr search agent --match "<case-insensitive-regex>" --json --latest 20
sivtr search terminal --match "<case-insensitive-regex>" --json --latest 20
```

### Search latest terminal errors

For "最新终端报错" / "刚才终端报错", search terminal records directly, then expand the newest shell ref.

```bash
sivtr search terminal --status failure --json --latest 1
```

If status metadata is unavailable or too sparse, broaden with an error regex:

```bash
sivtr search terminal --match "Error|error|failed|fatal|panic|Traceback|Exception|exit code|not found|External command failed|No such file or directory|permission denied|is not recognized" --json --latest 20
```

If the returned ref ends with a line number, remove the trailing line segment and run `sivtr show "<block-ref>" --json` before answering.

### Search terminal + AI memory for common errors

```bash
sivtr search terminal --match "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --json --latest 20
sivtr search agent --match "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --json --latest 20
```

### Rust failures

```bash
sivtr search terminal --match "error\\[E[0-9]+\\]|panicked|test result: FAILED|could not compile|borrow|lifetime" --json --latest 20
```

### JavaScript / TypeScript failures

```bash
sivtr search terminal --match "TypeError|ReferenceError|TS[0-9]+|npm ERR|pnpm|vite|webpack|ELIFECYCLE|failed" --json --latest 20
```

### Python failures

```bash
sivtr search terminal --match "Traceback|ModuleNotFoundError|ImportError|AssertionError|pytest|FAILED|Exception" --json --latest 20
```

### Previous decisions or AI discussion

```bash
sivtr search agent --match "lazy load|workspace TUI|metadata scan|decision|TODO|next step" --json --latest 20
```

### Search titles instead of content

```bash
sivtr search agent --match "workspace picker" --in session --json --latest 20
sivtr search terminal --match "cargo test" --in title --json --latest 20
```

### Provider-specific search

```bash
sivtr search codex --match "<query>" --json --latest 20
sivtr search claude --match "<query>" --json --latest 20
sivtr search pi --match "<query>" --json --latest 20
sivtr search opencode --match "<query>" --json --latest 20
```

### Compose filters from the request

Map request constraints to target selectors and filters instead of hard-coding
scenario-specific queries. Keep `--match` for the content/topic being searched.

```bash
sivtr search <provider> --match "<topic>" --last <duration> --json --latest 20
sivtr search <provider> --match "<topic>|<related-term>|<status-term>" --last <duration> --json --latest 30
sivtr search agent --match "<topic>" --in <content|title|session> --cwd <path> --json --latest 20
```

Examples of the mapping:

- "pi 中的 merge" -> target `pi`, match `merge`
- "最近两小时的终端报错" -> target `terminal`, options `--status failure --last 2h`
- "这个仓库上次的 CI 失败" -> target `terminal`, match `CI|failed`, option `--cwd <repo>`
- "标题里有 workspace picker" -> target `agent`, match `workspace picker`, option `--in session` or `--in title`

## JSON Handling

Treat `--json` output as structured evidence, not as a free-form transcript.

`sivtr search --json` returns a wrapper with `target`, optional `match`,
`field`, `cwd`, `match_count`, and `results`. Inspect these result fields first:

- `ref`: stable reference for follow-up expansion
- `kind`: `shell` or `ai`
- `timestamp`: how recent it is
- `title.session`: session title
- `title.dialogue`: dialogue or command block title, when available
- `content`: matched or extracted content

Expected result item shape:

```json
{
  "ref": "terminal/current/12/1",
  "kind": "shell",
  "timestamp": "...",
  "title": {
    "session": "current shell",
    "dialogue": "cargo test"
  },
  "content": "cargo test"
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
terminal/session/dialogue[/line]
provider/session/dialogue[/line]
```

Examples:

```bash
sivtr show "terminal/current/12" --json
sivtr show "pi/019e4f40/3" --json
```

## Token Budget

- Start with `--latest 20`.
- Expand at most 1-3 refs before answering unless the task requires a timeline.
- Prefer exact refs over broad repeated searches.
- If context is still missing after targeted search and expansion, ask the user.
