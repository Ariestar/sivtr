# Command Cookbook

Use these commands as starting points. Prefer small, targeted queries over dumping large histories.

## Core Model

`search`, `filter`, `nav`, `zoom`, `show`, `work records`, and `work parts` all take a source.

Source forms:

- `terminal`: terminal command records
- `agent`: AI/agent conversation records from all providers
- `codex`, `claude`, `cursor`, `opencode`, `openclaw`, `grok`, `hermes`, `pi`, `qoder`: one provider's conversation records
- `terminal/<session>/<record>`, `<provider>/<session>/<turn>`, or `<provider>/<session>/<turn>/p<part>` for input/output part refs
- `origin:body` for another local workspace name or a named remote, for example `desk:terminal`, `docs:codex/4`
- `@last`, `@name`, `@name[1]`, `@name[1,3]`, `@name[1..5]`, `@name[1..3,8]`
- `@` to read a WorkSet from stdin

WorkSets contain materialized `records` plus active `anchors`. Pipes move anchors; records are the backing store. WorkSet selector indexes are 1-based. Discrete selectors keep the requested order. `filter` narrows anchors, `nav` moves anchors, `var` remembers anchors, and `show` renders anchors.

Remote origins are registered with `sivtr remote add <alias> <invite>` or listed as local workspace names by `sivtr ws list`. Unregistered origins error.

## Search

`search` filters parts from a source and creates a WorkSet. It searches `WorkPart`s, then emits anchors at the current source granularity:

- record anchor in -> record anchor out
- part anchor in -> part anchor out

Every search saves the result to `@last`; `--save <name>` also saves it as `@name`.

Search is BM25-primary: a plain-text positional `QUERY` (no regex) ranks the whole source by relevance. `-m` / `--match` is an optional case-insensitive regex that bounds the set first; with both given, the regex bounds and the QUERY ranks. `--match` alone keeps the regex-filter behavior and its text doubles as the rank query. No QUERY and no `--match` = recency browse (`--latest 5` default).

```bash
sivtr s <source> [QUERY] \
  -m <case-insensitive-regex> \
  -i <content|title|session|input|output|command|all> \
  -v <case-insensitive-regex> \
  --kind <prompt|command|user|assistant|tool|tool_call|tool_result|skill|thinking|output|error> \
  --status <success|failure|unknown|fail> \
  --exit-code <code> \
  --min-duration <duration> \
  --max-duration <duration> \
  --sort <newest|relevance|oldest|duration|duration-asc|exit-code|exit-code-asc> \
  --cwd <path> \
  --last <duration> \
  --since <time> \
  --until <time> \
  --latest <n> \
  -l --limit <n> \
  --exclude-current \
  --save <name> \
  -f <full|timeline|compact|md|refs|workset>
```

Filters:

- `QUERY`: plain-text search query; BM25 ranks the source by these terms (no regex). Default sort becomes `relevance`.
- `-m` / `--match`: case-insensitive regex content filter that bounds the set before relevance ranking.
- `-v` / `--exclude`: case-insensitive regex exclusion filter on the same `-i` / `--in` search surface.
- `-i` / `--in`: part candidate/search field for `--match` and `--exclude`; default is `content`. Use `input`, `output`, `command`, or `all` to constrain which parts are searched.
- `--kind`: first-class part kind filter (`prompt`, `command`, `user`, `assistant`, `tool`, `tool_call`, `tool_result`, `skill`, `thinking`, `output`, `error`).
- `--status`: filter by command/record outcome.
- `--exit-code`: filter terminal records by exact process exit code.
- `--min-duration` / `--max-duration`: filter by duration (`500ms`, `2s`, `1m`, `1h`).
- `--sort`: sort results (`newest` default, `relevance` default with a QUERY or `--match`, plus `oldest`, `duration`, `duration-asc`, `exit-code`, `exit-code-asc`).
- `--cwd`: choose the workspace used to resolve current AI sessions.
- `--last`: relative time window (`30m`, `2h`, `7d`).
- `--since` / `--until`: bound search by RFC3339 time, Unix seconds/millis, or relative durations.
- `--latest`: return the latest N matching anchors. Defaults to 5 when neither `--latest` nor `--limit` is set (relevance sort ranks the whole set and skips the recency window).
- `-l` / `--limit`: cap printed results (hard ceiling after latest/sort).
- `--exclude-current` (`--other`): exclude the current agent session from agent searches.
- `--save`: name the result WorkSet.

Output formats:

- `full`: complete anchor content.
- `timeline`: timestamp, provider/source, ref, and title.
- `compact`: concise human-readable records.
- `md`: markdown export.
- `refs`: plain refs.
- `workset`: WorkSet JSON.
- `--json`: alias for `-f workset`.
- `--refs`: alias for `-f refs`.

Default output:

- Terminal stdout: `full`.
- Piped stdout: `workset`.

## Filter

`filter` applies the shared WorkSet filters to any source or piped WorkSet. It is the preferred way to narrow an existing WorkSet without re-running a broad provider search.

```bash
sivtr filter <source> \
  -m <case-insensitive-regex> \
  -v <case-insensitive-regex> \
  -i <content|title|session|input|output|command|all> \
  --parts \
  --kind <prompt|command|user|assistant|tool|tool_call|tool_result|skill|thinking|output|error> \
  --status <success|failure|unknown|fail> \
  --exit-code <code> \
  --last <duration> \
  --since <time> \
  --until <time> \
  --latest <n> \
  --limit <n> \
  --save <name> \
  -f <full|timeline|compact|md|refs|workset>
```

Examples:

```bash
sivtr s terminal --status fail --latest 20 --save failures --refs
sivtr filter @failures -m "panic|compile" --save focused --refs
sivtr filter @focused --parts --kind tool_result --refs
sivtr s terminal --json | sivtr filter @ -m "error" --refs
```

## Nav

`nav` moves anchors through the structural workspace. It does not default-expand children: child movement must specify a 1-based index with `>N`.

```text
<          parent
>N         Nth child, 1-based
+N         next sibling by N at the current level
-N         previous sibling by N at the current level
[A..B]     sibling window at the current level
~          containing session records
```

Examples:

```bash
sivtr nav @hit '<' --refs          # part -> record; record -> session records
sivtr nav @hit '>1' --refs         # record -> first child part
sivtr nav @hit '<+1>1' --refs      # part -> record -> next record -> first child
sivtr nav @hit '<[-2..+2]' --refs  # parent record context window
sivtr nav @hit '~' --refs          # containing session records
```

Use `zoom` when you simply want neighboring record context around hits. Use `nav` when the movement path matters.

## Var

`var` manages named WorkSet variables.

```bash
sivtr var set <name> [source]
sivtr var list
sivtr var rm <name>
sivtr var merge <name> <source>...
sivtr var drop <name> <source>...
sivtr var cleanup
```

Examples:

```bash
sivtr var set ctx @last
sivtr filter terminal -m "panic" --json | sivtr var set failures
sivtr var merge ctx @failures @last[1]
sivtr var drop ctx @noise
sivtr var list
```

`var merge` and `var drop` deduplicate by full anchor string and preserve first occurrence order.

- `sivtr filter` is available for narrowing an existing WorkSet with the same filter surface as `search`.
- `sivtr var` manages named WorkSet variables: `set`, `list`, `rm`, `merge`, `drop`, and `cleanup`.
- `sivtr nav` moves anchors deterministically with `<`, `>N`, `+N`, `-N`, `[A..B]`, and `~`; there is no implicit child expansion.

## Search Remote / Other-Workspace Origins

When the user refers to a teammate machine, a named remote, or another local workspace by name:

```bash
sivtr ws list
sivtr remote list
sivtr s desk:terminal --status fail --latest 5 --refs
sivtr s desk:agent -m "decision|failed|TODO" --latest 20 --save remote_hits --refs
sivtr show desk:terminal/session_42/3/p1 --full
sivtr show docs:codex/4 --full
```

If the origin is unknown, say so and stop. Do not invent remotes. Sharing/adding remotes (`sivtr share`, `sivtr share invite`, `sivtr remote add`) is an explicit user action, not a retrieval default.

## General Search

```bash
sivtr s agent -m "<case-insensitive-regex>" --latest 20 --refs
sivtr s terminal -m "<case-insensitive-regex>" --latest 20 --refs
sivtr show @last -f timeline
```

## Search Latest Terminal Errors

```bash
sivtr s terminal --status fail --latest 1 --refs
```

If status metadata is sparse:

```bash
sivtr s terminal -m "Error|error|failed|fatal|panic|Traceback|Exception|exit code|not found|External command failed|No such file or directory|permission denied|is not recognized" --latest 20 --refs
```

Expand the newest match:

```bash
sivtr zoom @last[1] -C 2 --save error_ctx --refs
sivtr show @error_ctx --full
```

## Terminal Metadata Filters

```bash
sivtr s terminal --status fail --latest 5 --refs
sivtr s terminal --exit-code 101 --latest 20 --refs
sivtr s terminal --min-duration 2s --sort duration --latest 20 -f timeline
```

## Search Terminal + AI Memory for Common Errors

```bash
sivtr s terminal -m "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --latest 20 --refs
sivtr s agent -m "error|failed|panic|Traceback|Exception|exit code|could not compile|FAILED" --latest 20 --refs
```

## Language Failure Queries

```bash
sivtr s terminal -m "error\\[E[0-9]+\\]|panicked|test result: FAILED|could not compile|borrow|lifetime" --latest 20 --refs
sivtr s terminal -m "TypeError|ReferenceError|TS[0-9]+|npm ERR|pnpm|vite|webpack|ELIFECYCLE|failed" --latest 20 --refs
sivtr s terminal -m "Traceback|ModuleNotFoundError|ImportError|AssertionError|pytest|FAILED|Exception" --latest 20 --refs
```

## Previous Decisions or AI Discussion

```bash
sivtr s agent -m "decision|TODO|next step|blocked|test result|passed|failed" --latest 20 --save history --refs
sivtr filter @history -m "<topic>" --save topic_history --refs
sivtr zoom @topic_history[1] -C 2 --save topic_ctx --refs
sivtr show @topic_ctx --full
```

## Search Titles Instead of Content

```bash
sivtr s agent -m "workspace picker" -i session --latest 20 --refs
sivtr s terminal -m "cargo test" -i title --latest 20 --refs
```

## Provider-Specific Search

```bash
sivtr s codex -m "<query>" --latest 20 --refs
sivtr s claude -m "<query>" --latest 20 --refs
sivtr s cursor -m "<query>" --latest 20 --refs
sivtr s opencode -m "<query>" --latest 20 --refs
sivtr s openclaw -m "<query>" --latest 20 --refs
sivtr s hermes -m "<query>" --latest 20 --refs
sivtr s grok -m "<query>" --latest 20 --refs
sivtr s pi -m "<query>" --latest 20 --refs
```

Part kinds include dialogue (`user`, `assistant`, `tool_call`, `tool_result`, …) and structure (`skill`, `thinking`). Default content search covers dialogue turns, terminal output, tool results, and thinking; tool-call payloads and skill text need `--kind tool_call` / `--kind skill` or `-i all`.

## Compose Filters from the Request

Map request constraints to source selectors and filters. Put the content/topic being searched in the positional `QUERY` for relevance ranking, or in `-m` / `--match` as a regex bound.

```bash
sivtr s <provider> "<topic>" --last <duration> --latest 20 --refs
sivtr s <provider> -m "<topic>|<related-term>|<status-term>" --last <duration> --latest 30 --refs
sivtr s agent "<topic>" -i <content|title|session> --cwd <path> --latest 20 --refs
```

Examples:

- "pi 中的 merge" -> source `pi`, match `merge`
- "最近两小时的终端报错" -> source `terminal`, options `--status failure --last 2h`
- "这个仓库上次的 CI 失败" -> source `terminal`, match `CI|failed`, option `--cwd <repo>`
- "标题里有 workspace picker" -> source `agent`, match `workspace picker`, option `--in session` or `--in title`

## Chaining WorkSets

Pipeline chaining uses `@` as stdin source. Intermediate commands can omit `-f`; piped stdout emits WorkSet JSON automatically.

```bash
sivtr s terminal --status fail -m "error|failed|拒绝访问" \
  | sivtr filter @ -v "example|sample|sttop" -i title \
  | sivtr zoom @ -C 1 \
  | sivtr show @ -f timeline
```

Show part-level output matches from the current WorkSet:

```bash
sivtr s pi "git push" --latest 10 \
  | sivtr work parts @ --kind tool_result \
  | sivtr filter @ -m "main -> main" \
  | sivtr show @ --full
```

If an intermediate command prints `--refs`, do not pipe that text into `@`; `@` expects WorkSet JSON from piped stdout. Use `@last` or omit `--refs` in pipelines.

Named WorkSets are useful for multi-step retrieval:

```bash
sivtr s agent -m "decision|TODO|next step" --latest 20 --save hits --refs
sivtr filter @hits -m "<topic>" --save narrowed --refs
sivtr nav @narrowed[1] '<[-1..+1]' --refs
sivtr zoom @narrowed[1] -C 2 --save ctx --refs
sivtr show @ctx --full
```

## Zoom

`zoom` maps any source anchor to its parent record, expands to neighboring records in the same session, and creates a new WorkSet with record anchors.

```bash
sivtr zoom <source> -C <n> --save <name> --refs
sivtr zoom @last[1] --before 3 --after 1 -f timeline
sivtr zoom @ -C 2 | sivtr show @ --full
```

Options:

- `-C` / `--context`: set both before and after.
- `--before`: records before each source record.
- `--after`: records after each source record.
- `--save`: name the expanded WorkSet.

## Show

`show` displays any source at anchor granularity: record anchors show records, part anchors show only that part.

```bash
sivtr show @last --full
sivtr show @last[1,3] -f timeline
sivtr show @ctx -f md
sivtr show "terminal/current/12" --full
sivtr show "pi/019e4f40/3" --full
sivtr show "codex/abc123/2/p1" --full
sivtr show "desk:terminal/session_42/3" --full
```

Refs/selectors have this shape:

```text
[origin:]terminal/session/record
[origin:]provider/session/turn
[origin:]provider/session/turn/p<part>
```

The `record` / `turn` segments may be concrete numbers or selector lists/ranges when used as command input, for example `3-5,7`. Output refs remain concrete anchors. Part refs are `p` followed by a 1-based part index.

## Token Budget

- Search defaults to `--latest 5` when neither `--latest` nor `--limit` is set. Raise intentionally (`--latest 20`) when scanning.
- Use `--refs` or `-f timeline` for first-pass inspection.
- Expand at most 1-3 records before answering unless the task requires a timeline.
- Prefer exact refs and WorkSet selectors over broad repeated searches.
- If context is still missing after targeted search and expansion, ask the user.

## Copy by Ref

Copy content behind an exact address to clipboard. `copy` takes the address first (same source/ref language as search/show), then an optional dialogue selector:

```bash
sivtr copy codex/019e4f40/3
sivtr copy terminal/session_42/5 --print
sivtr copy pi/abc123/2/p1 --lines "1:10"
sivtr copy desk:terminal/session_42/3 --print
```

Supports `--print`, `--regex`, `--lines`, and `--prompt` options, plus `in` / `out` / `cmd` projections.

## Work Traversal

Explore workspace records at session, record, and part granularity:

```bash
sivtr work sessions --json
sivtr work records codex/019e4f40 --refs
sivtr work records @last[1] --refs
sivtr work parts codex/019e4f40/3 --kind tool_result --refs
sivtr work parts @ --kind tool --json
```

`work records` projects any source anchors to parent record anchors. `work parts` projects source anchors to part anchors and supports `--kind` and `-m` / `--match` filtering. For new pipelines, prefer `sivtr filter --parts ...` when you want the shared filter surface.

## Diagnostics

When `sivtr` commands fail or return no results, check the environment:

```bash
sivtr doctor
sivtr init show
```

Common issues:

- No terminal results -> `sivtr init show` to verify hooks, then restart terminal.
- No provider results -> `sivtr doctor` to check session discovery.
- Clipboard not working -> `sivtr doctor` reports clipboard availability.
