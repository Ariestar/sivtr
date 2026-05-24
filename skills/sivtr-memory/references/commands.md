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
  --exit-code <code> \
  --min-duration <duration> \
  --max-duration <duration> \
  --sort <newest|oldest|duration|duration-asc|exit-code|exit-code-asc> \
  --cwd <path> \
  --last <duration> \
  --since <time> \
  --until <time> \
  --latest <n> \
  --limit <n> \
  --format json
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
- `--exit-code`: filter terminal records by exact process exit code.
- `--min-duration` / `--max-duration`: filter by duration (`500ms`, `2s`, `1m`, `1h`).
- `--sort`: sort results (`newest`, `oldest`, `duration`, `duration-asc`, `exit-code`, `exit-code-asc`).
- `--cwd`: choose the workspace used to resolve current AI sessions. Omit it
  when already running in the target repo.
- `--last`: relative time window (`30m`, `2h`, `7d`).
- `--since` / `--until`: bound search by RFC3339 time, Unix seconds/millis, or
  relative durations.
- `--latest`: return the latest N matching records.
- `--limit`: cap printed results; use when you want a display cap different
  from `--latest`.
- `--format`: output view (`timeline`, `compact`, `md`, or `json`). Prefer `json` when a program must parse fields; use the readable formats freely for agent reasoning, summaries, and handoffs.

### General search

```bash
sivtr search agent --match "<case-insensitive-regex>" --format json --latest 20
sivtr search terminal --match "<case-insensitive-regex>" --format json --latest 20
```

### Search latest terminal errors

For "最新终端报错" / "刚才终端报错", search terminal records directly, then expand the newest shell ref.

```bash
sivtr search terminal --status failure --format json --latest 1
```

If status metadata is unavailable or too sparse, broaden with an error regex:

```bash
sivtr search terminal --match "Error|error|failed|fatal|panic|Traceback|Exception|exit code|not found|External command failed|No such file or directory|permission denied|is not recognized" --format json --latest 20
```

If the returned ref ends with a line number, remove the trailing line segment and run `sivtr show "<block-ref>" --json` before answering.

### Terminal metadata filters

```bash
sivtr search terminal --status failure --format json --latest 5
sivtr search terminal --exit-code 101 --format json --latest 20
sivtr search terminal --min-duration 2s --sort duration --format json --latest 20
```

### Search terminal + AI memory for common errors

```bash
sivtr search terminal --match "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --format json --latest 20
sivtr search agent --match "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --format json --latest 20
```

### Rust failures

```bash
sivtr search terminal --match "error\\[E[0-9]+\\]|panicked|test result: FAILED|could not compile|borrow|lifetime" --format json --latest 20
```

### JavaScript / TypeScript failures

```bash
sivtr search terminal --match "TypeError|ReferenceError|TS[0-9]+|npm ERR|pnpm|vite|webpack|ELIFECYCLE|failed" --format json --latest 20
```

### Python failures

```bash
sivtr search terminal --match "Traceback|ModuleNotFoundError|ImportError|AssertionError|pytest|FAILED|Exception" --format json --latest 20
```

### Previous decisions or AI discussion

```bash
sivtr search agent --match "lazy load|workspace TUI|metadata scan|decision|TODO|next step" --format json --latest 20
```

### Search titles instead of content

```bash
sivtr search agent --match "workspace picker" --in session --format json --latest 20
sivtr search terminal --match "cargo test" --in title --format json --latest 20
```

### Provider-specific search

```bash
sivtr search codex --match "<query>" --format json --latest 20
sivtr search claude --match "<query>" --format json --latest 20
sivtr search pi --match "<query>" --format json --latest 20
sivtr search opencode --match "<query>" --format json --latest 20
```

### Compose filters from the request

Map request constraints to target selectors and filters instead of hard-coding
scenario-specific queries. Keep `--match` for the content/topic being searched.

```bash
sivtr search <provider> --match "<topic>" --last <duration> --format json --latest 20
sivtr search <provider> --match "<topic>|<related-term>|<status-term>" --last <duration> --format json --latest 30
sivtr search agent --match "<topic>" --in <content|title|session> --cwd <path> --format json --latest 20
```

Examples of the mapping:

- "pi 中的 merge" -> target `pi`, match `merge`
- "最近两小时的终端报错" -> target `terminal`, options `--status failure --last 2h`
- "这个仓库上次的 CI 失败" -> target `terminal`, match `CI|failed`, option `--cwd <repo>`
- "标题里有 workspace picker" -> target `agent`, match `workspace picker`, option `--in session` or `--in title`

## Format Handling

Search formats are interchangeable views over the same result set. Use `json` when you need structured fields; use `timeline`, `compact`, or `md` when the agent needs to reason over order, summarize work, or draft a handoff.

`sivtr search --format json` returns a wrapper with `target`, optional `match`,
`field`, `cwd`, `count`, and `results`. Inspect these result fields first:

- `ref`: stable reference for follow-up expansion
- `timestamp`: how recent it is
- `dialogue`: dialogue or command block title
- `status`: `success`, `failure`, or `unknown`
- `exit_code`: terminal process exit code when available
- `duration_ms`: command/turn elapsed time when available

Expected result item shape:

```json
{
  "ref": "terminal/current/12/1",
  "timestamp": "...",
  "dialogue": "cargo test"
}
```

Use `ref` for precise follow-up. Search output is intentionally compact; use
`show` when you need exact content.

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
