# Command Cookbook

Use these commands as starting points. This file is the single source for `sivtr` command syntax.
Prefer small, targeted queries over dumping large histories.

## Search Commands

### Search option reference

`sivtr search` uses a target selector plus filters. The target decides where to
search; filters decide which records match. Content search is a filter via
`--match`.

```bash
sivtr s <target> \
  -m <case-insensitive-regex> \
  -i <content|title|session|input|output|command|all> \
  -v <case-insensitive-regex> \
  --status <success|failure|unknown|fail> \
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
  -f <timeline|compact|md|refs|json>
```

Targets:

- `terminal`: terminal command records.
- `agent`: AI/agent conversation records from all providers.
- `codex`, `claude`, `pi`, `opencode`: one provider's conversation records.
- `terminal/<session>/<record>/<line>` or `<provider>/<session>/<turn>/<line>`:
  ref-like narrowing. Trailing segments are optional. `*` means wildcard.
- Record and line segments accept 1-based selectors: `3`, `3-5`, `3,7`, or
  `3-5,7`. Use these selectors to narrow search scope; search results still
  return concrete refs such as `pi/<session>/3/12`.

Filters:

- `-m` / `--match`: case-insensitive regex content filter.
- `-v` / `--exclude`: case-insensitive regex exclusion filter on the same `-i`/`--in` search surface.
- `-i` / `--in`: field for `--match` and `--exclude`; default is `content`.
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
- `-f` / `--format`: output view (`timeline`, `compact`, `md`, `refs`, or `json`). Prefer default JSON / `--json` when a program must parse fields; use the readable formats freely for agent reasoning, summaries, and handoffs.

### General search

```bash
sivtr s agent -m "<case-insensitive-regex>" --json --latest 20
sivtr s terminal -m "<case-insensitive-regex>" --json --latest 20
```

### Search latest terminal errors

For "最新终端报错" / "刚才终端报错", search terminal records directly, then expand the newest shell ref.

```bash
sivtr s terminal --status fail --json --latest 1
```

If status metadata is unavailable or too sparse, broaden with an error regex:

```bash
sivtr s terminal -m "Error|error|failed|fatal|panic|Traceback|Exception|exit code|not found|External command failed|No such file or directory|permission denied|is not recognized" --json --latest 20
```

If the returned ref ends with a line number, remove the trailing line segment and run `sivtr show "<block-ref>" --json` before answering.

### Terminal metadata filters

```bash
sivtr s terminal --status fail --json --latest 5
sivtr s terminal --exit-code 101 --json --latest 20
sivtr s terminal --min-duration 2s --sort duration --json --latest 20
```

### Search terminal + AI memory for common errors

```bash
sivtr s terminal -m "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --json --latest 20
sivtr s agent -m "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --json --latest 20
```

### Rust failures

```bash
sivtr s terminal -m "error\\[E[0-9]+\\]|panicked|test result: FAILED|could not compile|borrow|lifetime" --json --latest 20
```

### JavaScript / TypeScript failures

```bash
sivtr s terminal -m "TypeError|ReferenceError|TS[0-9]+|npm ERR|pnpm|vite|webpack|ELIFECYCLE|failed" --json --latest 20
```

### Python failures

```bash
sivtr s terminal -m "Traceback|ModuleNotFoundError|ImportError|AssertionError|pytest|FAILED|Exception" --json --latest 20
```

### Previous decisions or AI discussion

```bash
sivtr s agent -m "lazy load|workspace TUI|metadata scan|decision|TODO|next step" --json --latest 20
```

### Search titles instead of content

```bash
sivtr s agent -m "workspace picker" -i session --json --latest 20
sivtr s terminal -m "cargo test" -i title --json --latest 20
```

### Provider-specific search

```bash
sivtr s codex -m "<query>" --json --latest 20
sivtr s claude -m "<query>" --json --latest 20
sivtr s pi -m "<query>" --json --latest 20
sivtr s opencode -m "<query>" --json --latest 20
```

### Compose filters from the request

Map request constraints to target selectors and filters instead of hard-coding
scenario-specific queries. Keep `--match` for the content/topic being searched.

```bash
sivtr s <provider> -m "<topic>" --last <duration> --json --latest 20
sivtr s <provider> -m "<topic>|<related-term>|<status-term>" --last <duration> --json --latest 30
sivtr s agent -m "<topic>" -i <content|title|session> --cwd <path> --json --latest 20
```

Examples of the mapping:

- "pi 中的 merge" -> target `pi`, match `merge`
- "最近两小时的终端报错" -> target `terminal`, options `--status failure --last 2h`
- "这个仓库上次的 CI 失败" -> target `terminal`, match `CI|failed`, option `--cwd <repo>`
- "标题里有 workspace picker" -> target `agent`, match `workspace picker`, option `--in session` or `--in title`

## Chaining Searches

Search defaults to JSON, so intermediate pipeline stages should usually omit
`-f`. A downstream `sivtr s ...` reads the previous search JSON from stdin and
filters within those result refs. Choose the human display format only on the
last stage.

```bash
sivtr s terminal --status fail -m "error|failed|拒绝访问" \
  | sivtr s terminal -v "example|sample|sttop" -i title -f timeline
```

Use `--refs` only when you explicitly want a plain ref list for humans, files,
or non-sivtr tools.

## Format Handling

Search formats are interchangeable views over the same result set. Use default JSON / `--json` when you need structured fields; use `-f timeline`, `-f compact`, or `-f md` when the agent needs to reason over order, summarize work, or draft a handoff.

`sivtr s --json` returns a wrapper with `target`, optional `match`,
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

Refs/selectors have this shape:

```text
terminal/session/dialogue[/line]
provider/session/dialogue[/line]
```

The `dialogue`/`line` segments may be concrete numbers or selector lists/ranges
when used as command input, for example `3-5,7` or `5-7,10`. Output refs remain
concrete anchors.

Examples:

```bash
sivtr show "terminal/current/12" --json
sivtr show "pi/019e4f40/3" --json
sivtr show "pi/019e4f40/3-5,7" --json
sivtr show "pi/019e4f40/3/5-7,10" --json
```

## Token Budget

- Start with `--latest 20`.
- Expand at most 1-3 refs before answering unless the task requires a timeline.
- Prefer exact refs over broad repeated searches.
- If context is still missing after targeted search and expansion, ask the user.

## Diagnostics

When `sivtr` commands fail or return no results, check the environment:

```bash
sivtr doctor        # Check binary, config, session logs, hooks, providers, clipboard
sivtr init show     # Show which shell hooks are installed
```

Common issues:
- No terminal results → `sivtr init show` to verify hooks, then restart terminal
- No provider results → `sivtr doctor` to check session discovery
- Clipboard not working → `sivtr doctor` reports clipboard availability
