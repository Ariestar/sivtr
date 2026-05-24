# Command Cookbook

Use this file as the single source for `sivtr` syntax. The recommended path is `search -> show -> copy`.

Work records come in two kinds:

- `shell` for terminal records
- `ai` for agent records

## Search

Start small and stay targeted.

```bash
sivtr search "error|failed|panic|Traceback|Exception|exit code|FAILED" --json -l 20
sivtr search "decision|TODO|next step|blocked" --scope dialogue --provider codex --json -l 20
sivtr search "workspace picker" --scope session --json -l 20
sivtr search "build error" --recent 2h --json -l 20
sivtr search "panic" --since 2026-05-23T00:00:00Z --until 2026-05-24 --json
```

Search defaults:

- current workspace AI sessions
- current terminal capture, when available
- case-insensitive regex matching

Useful flags:

- `--scope content|dialogue|session`
- `--provider all|codex|claude|opencode|pi`
- `--cwd PATH`
- `--recent COUNT|DURATION` such as `20`, `30m`, `2h`, or `7d`
- `--since TIME` and `--until TIME`
- `-l, --limit N` with a default of `20`
- `--json` for machine use

Time filters accept RFC3339, Unix seconds or milliseconds, `YYYY-MM-DD`, `YYYY-MM-DD HH:MM:SS`, `YYYY-MM-DDTHH:MM:SS`, or relative durations through `--recent`.

## JSON Output

`sivtr search --json` returns a wrapper with:

- `query`
- `scope`
- `cwd`
- `match_count`
- `results`

Each result has:

- `ref`
- `kind` (`shell` or `ai`)
- `timestamp`
- `title.session`
- `title.dialogue`
- `content`

Example:

```json
{
  "query": "panic",
  "scope": "content",
  "cwd": "/home/shiro/Projects/sivtr",
  "match_count": 1,
  "results": [
    {
      "ref": "terminal/current/12/8",
      "kind": "shell",
      "timestamp": "2026-05-24T10:11:12Z",
      "title": {
        "session": "current",
        "dialogue": "cargo test"
      },
      "content": "test result: FAILED"
    }
  ]
}
```

## Show

Use `show` to expand an exact ref.

```bash
sivtr show "terminal/current/12"
sivtr show "terminal/current/12/8"
sivtr show "pi/019e4f40/3"
sivtr show "claude/abcdef12/2/4"
```

Ref shapes:

- `terminal/<session>/<record>[/line]`
- `<provider>/<session>/<turn>[/line]`

`show --json` returns the same item shape as search results, with exact content for the selected ref or line.

## Copy

Use `copy` when you need the raw text behind recent terminal blocks or a provider session.

Prefer `--print` for agents.

```bash
sivtr copy out 1 --print
sivtr copy 1 --print
sivtr copy cmd 1..10 --print
```

Use interactive picker flags only when the user wants interaction.
