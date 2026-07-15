---
title: Known Issues
description: Tracked polish items deferred from the agent-registry unification work.
---

Working list of known product gaps. Prefer fixing these over adding new surface area.

## Copy CLI shape (deferred)

`sivtr copy` agent entry is registry-driven via clap `external_subcommand`, not a per-provider enum. That is correct for “no hard-coded provider list”, but the CLI shape still mixes concerns:

| Issue | Detail |
| --- | --- |
| Dual token meaning | First free token is either a registry provider (`codex`) or a terminal selector (`3`, `2..4`). Unknown non-provider names become terminal selectors instead of a clear error. |
| Flag types split | Bare `copy` uses `CopyFlagArgs` (no positional selector). `copy in/out/cmd` still use `CopyCommonArgs` with optional selector. |
| Parent flag merge | `copy 3 --print` may combine parent flags with trailing external flags via `merge_copy_flags`. |
| Nested help | `copy cursor --help` / `copy codex out --help` show the nested parser help, not a single polished agent help page. |

### Preferred cleanup (later)

Pick one product shape and delete the other:

1. **Strict split (cleaner):**  
   `sivtr copy agent <provider> …` for agents; terminal selectors only as numbers / ranges on bare `copy`.
2. **Keep current UX:** lock rules with tests, validate terminal selectors more strictly, unify flag structs, fix help strings.

Do not reintroduce per-provider `CopySubcommand` variants.

## Structured parts (landed baseline)

Evidence channels are typed on `WorkPartKind` / `AgentBlockKind` with methods for extension:

- `is_dialogue` / `is_structure` / `default_io` / `as_agent_block_kind` / `format_block` (markers)
- Structure markers: `<:tool:name call:>` / `<:tool:name result:>`, `<:skill:name:>`, `<:thinking:>`
- MCP is **not** a separate kind — MCP tools are normal tools (label = tool name)
- Last turn **keeps** structure; do not reintroduce stripping from records
- **Default content search** (`-i content`) skips `is_structure()` parts so they don't pollute full-text hits; use `--kind tool_call|skill|thinking` or `-i all` to include them

### Provider emit gaps

| Kind | Model | Default search | Parse emit |
| --- | --- | --- | --- |
| Tool* | yes | excluded unless opted in | most providers |
| Skill | yes | excluded unless opted in | mainly inlined `<skill name>` |
| Thinking | yes | excluded unless opted in | Claude/Pi/OpenCode reasoning now kept |

Adding a new structure channel: extend enum methods first, then CLI kind aliases, then provider emit. Avoid Markdown `## Tool Call` for structure.

## Workspace filter

Shared policy lives in `filter_sessions_by_workspace` (unbound keep + path / git-remote match). All agent list paths use it (JSONL helper, Hermes, OpenClaw, OpenCode, Codex configured dirs). Reimplementing cwd filtering per provider is a regression.

## Doctor: agent hosts vs MCP

Doctor splits two checks:

1. **agent hosts** — which agent products appear installed (`detected_hosts()`: config dir / file present). Informational; empty is Manual, not Fail.
2. **MCP registration** — whether a real `sivtr` entry exists in host config (`registered_targets()`). Fail if missing; `--fix` runs install then re-checks entries.

Do not collapse these into one “registered for …” line that only means host presence.

## Remote / iroh (review notes)

Not necessarily open bugs; watchpoints for future hardening:

- Remote protocol is one request/response per bi-stream connection (no multiplexing). Fine for current use; chatty clients pay connect cost each time.
- Invite redeem stores peer endpoint JSON from the ticket; if the peer’s dialable addresses change, remount / re-invite may be needed.
- `Source` responses can be large; `MAX_MESSAGE_SIZE` is 64 MiB — still a blast radius if a share root is huge.
- Local control is localhost + token in `daemon.json`; file permissions on Windows are not restricted the same way as Unix `0600`.
- Redaction is best-effort regex, not a security boundary (documented in code).
